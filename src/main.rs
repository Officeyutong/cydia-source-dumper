use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Parser;
use flexi_logger::{opt_format, Logger};
use log::{debug, info, warn};
use rand::Rng;
use reqwest::{
    header::{self, HeaderValue},
    Client,
};

pub type Result<T> = anyhow::Result<T>;
use anyhow::anyhow;
use tokio::sync::Semaphore;
use walkdir::WalkDir;
use zip::ZipWriter;

use crate::{
    hash::MyHash,
    util::{download, extract_file},
};
mod cmd;
mod hash;
mod util;
#[derive(Clone)]
pub struct AppState {
    pub client: Client,
    pub save_root: PathBuf,
    pub root_url: url::Url,
    pub semaphore: Arc<Semaphore>,
}
impl AppState {
    fn join_url<T: Into<String>>(&self, u: T) -> url::Url {
        let v = self.root_url.join(&u.into()).unwrap();
        v
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    // console_subscriber::ConsoleLayer::builder()
    //     // set how long the console will retain data from completed tasks
    //     .retention(std::time::Duration::from_secs(60))
    //     // set the address the server is bound to
    //     .server_addr(([127, 0, 0, 1], 6669))
    //     // ... other configurations ...
    //     .init();
    let arg = crate::cmd::Args::parse();
    Logger::try_with_str(if arg.debug { "debug" } else { "info" })
        .unwrap()
        .format(opt_format)
        .log_to_stdout()
        .start()
        .expect("Failed to start logger!");
    debug!("{:#?}", arg);
    let client = reqwest::ClientBuilder::new()
        .default_headers({
            let mut headers = header::HeaderMap::new();
            headers.insert(
                "X-Cydia-ID",
                HeaderValue::from_str(arg.cydia_id.as_str())
                    .map_err(|e| anyhow!("非法CydiaID: {}, {}", arg.cydia_id.as_str(), e))?,
            );
            headers.insert(
                "X-Firmware",
                HeaderValue::from_str(arg.firmware.as_str())
                    .map_err(|e| anyhow!("非法Firmware: {}, {}", arg.firmware.as_str(), e))?,
            );
            headers.insert(
                "X-Machine",
                HeaderValue::from_str(arg.machine.as_str())
                    .map_err(|e| anyhow!("非法Machine: {}, {}", arg.machine.as_str(), e))?,
            );
            headers.insert(
                "X-Unique-ID",
                HeaderValue::from_str(arg.unique_id.as_str())
                    .map_err(|e| anyhow!("非法X-Unique-ID: {}, {}", arg.unique_id.as_str(), e))?,
            );
            debug!("Header: {:#?}", headers);
            headers
        })
        .build()?;
    let save_buf = PathBuf::from(arg.save_dir.as_str());
    if !save_buf.exists() {
        std::fs::create_dir(save_buf.as_path())
            .map_err(|e| anyhow!("创建保存目录时发生失败: {}", e))?;
    }
    let state = AppState {
        client,
        save_root: save_buf,
        root_url: {
            let mut s = arg.repo_url.clone();
            if !s.ends_with("/") {
                s.push('/');
            }
            url::Url::parse(&s).map_err(|e| anyhow!("非法URL: {}, {}", arg.repo_url, e))?
        },
        semaphore: Arc::new(Semaphore::new(arg.worker as usize)),
    };
    info!("下载发布信息文件中..");
    let release_bytes = state
        .client
        .get(state.join_url("/Release"))
        .send()
        .await?
        .bytes()
        .await?;
    tokio::fs::write(state.save_root.join("Release"), release_bytes.to_vec()).await?;
    info!("发布信息文件已下载!");

    let things_to_try = [
        "Packages",
        "Packages.xz",
        "Packages.gz",
        "Packages.bz2",
        "Packages.lzma",
    ];
    let mut download_ok = vec![];
    for file in things_to_try.clone() {
        tokio::fs::remove_file(state.save_root.join(file))
            .await
            .ok();
        info!("尝试 {} 中..", file);
        match state.client.get(state.join_url(file)).send().await {
            Ok(v) => {
                if let Err(e) = v.error_for_status_ref() {
                    warn!("文件 {} 下载失败: {}", file, e);
                    continue;
                }
                match v.bytes().await {
                    Ok(b) => {
                        if let Err(e) =
                            tokio::fs::write(state.save_root.join(file), b.to_vec()).await
                        {
                            warn!("文件 {} 写入失败: {}", file, e);
                            continue;
                        }
                        download_ok.push(file);
                        info!("文件 {} 下载成功", file);
                        // .map_err(|e| anyhow!)?
                    }
                    Err(e) => {
                        warn!("文件 {} 获取失败: {}", file, e);
                    }
                };
            }
            Err(e) => {
                warn!("文件 {} 下载失败: {}", file, e);
                continue;
            }
        };
    }
    if download_ok.is_empty() {
        return Err(anyhow!(
            "没有找到可用的包索引文件！(已尝试 {:?} )\n您可以尝试{}",
            things_to_try,
            state
                .join_url("/dists/stable/main/binary-iphoneos-arm")
                .to_string()
        ));
    }
    let ext = extract_file(&state, *download_ok.last().unwrap())?;
    info!("解压信息中..");
    let s = unsafe { String::from_utf8_unchecked(ext) };
    let parsed = debcontrol::parse_str(&s).map_err(|e| anyhow!("非法deb控制文件: {}", e))?;
    // std::fs::write("qaq.txt", format!("{:#?}", parsed))?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
    for para in parsed.iter() {
        let map = BTreeMap::from_iter(para.fields.iter().map(|v| {
            (
                String::from(v.name.to_lowercase()),
                String::from(v.value.as_str()),
            )
        }));
        let local_tx = tx.clone();
        let local_state = state.clone();
        let local_client = state.client.clone();
        let url = state.join_url(map.get("filename").unwrap()).to_string();
        let filename = map.get("filename").unwrap().to_string();
        let bundle_id = map.get("package").unwrap().to_owned();
        let tweak_name = map.get("name").unwrap().to_owned();
        let checksum = if let Some(v) = map.get("md5sum") {
            MyHash::MD5(v.into())
        } else if let Some(v) = map.get("sha1") {
            MyHash::SHA1(v.into())
        } else if let Some(v) = map.get("sha256") {
            MyHash::SHA256(v.into())
        } else {
            info!("包 {}, {} 不存在被支持的校验和方式", bundle_id, tweak_name);
            MyHash::None
        };

        let mut rng = rand::thread_rng();
        let v = rng.gen_range(1000..=10000);
        tokio::spawn(async move {
            if let Err(e) = download(
                &local_state,
                local_client,
                &url,
                &filename,
                &tweak_name,
                &bundle_id,
                checksum,
                v,
            )
            .await
            {
                use log::error;
                error!("{}", e);
                local_tx.send(false).unwrap();
            } else {
                local_tx.send(true).unwrap();
            }
        });
    }
    let mut ok_cnt = 0;
    let mut fail_cnt = 0;
    while let Some(v) = rx.recv().await {
        debug!("{}", v);
        if v {
            ok_cnt += 1;
        } else {
            fail_cnt += 1;
        }
        if fail_cnt > arg.max_fail_count {
            return Err(anyhow!("失败任务数超出阈值，已强制终止!"));
        }
        if ok_cnt + fail_cnt == parsed.len() as u32 {
            break;
        }
    }
    info!(
        "下载完成！共成功 {} 个文件，失败 {} 个文件",
        ok_cnt, fail_cnt
    );
    if arg.pack {
        info!("打包中..");
        let save_dir = state.save_root.clone();
        let save_file = PathBuf::from(format!("{}.zip", arg.save_dir));

        let resp: Result<()> = tokio::task::spawn_blocking(move || {
            let prefix = save_dir.as_os_str().to_str().unwrap();
            let out_file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(save_file)?;
            let mut writer = ZipWriter::new(out_file);
            for file in WalkDir::new(save_dir.clone()) {
                let file = file?;
                let path = file.path();
                let name = path.strip_prefix(Path::new(prefix)).unwrap();
                if path.is_file() {
                    info!("添加: {:?} 到 {:?}", path, name);
                    writer.start_file(
                        name.as_os_str().to_str().unwrap(),
                        zip::write::FileOptions::default()
                            .compression_method(zip::CompressionMethod::Zstd),
                    )?;
                    let mut opened_file = std::fs::OpenOptions::new().read(true).open(path)?;
                    io::copy(&mut opened_file, &mut writer)?;
                }
            }
            writer.finish()?;
            return Ok(());
        })
        .await?;
        resp?;
    }
    Ok(())
}
