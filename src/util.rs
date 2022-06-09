use std::{
    io::{BufWriter, Read},
    time::Duration,
};

use anyhow::anyhow;
use gzip::GzipReader;
use log::{debug, error, info};
use lzma_rs::{lzma_decompress, xz_decompress};
use reqwest::Client;
use tokio::io::AsyncWriteExt;

use crate::{hash::MyHash, AppState};
pub fn extract_file<'a, T: Into<&'a str>>(state: &AppState, filename: T) -> crate::Result<Vec<u8>> {
    let filename: &str = filename.into();
    let s = filename.split(".").collect::<Vec<&str>>();
    debug!("Split: {:?}", s);
    let input_bytes = std::fs::read(state.save_root.join(filename))?;
    let mut input_buf = &input_bytes[..];
    let mut out_buf = BufWriter::new(vec![]);
    match s[..] {
        ["Release"] => Ok(input_bytes),
        [_, "bz2"] => {
            let mut reader = bzip2_rs::DecoderReader::new(&mut input_buf);
            reader.read_to_end(&mut out_buf.get_mut())?;
            Ok(out_buf.into_inner()?)
        }
        [_, "gz"] => {
            let mut reader = GzipReader::new(&mut input_buf);
            let mem = reader.read_member()?;
            Ok(mem.data)
        }
        [_, "lzma"] => {
            lzma_decompress(&mut input_buf, &mut out_buf)
                .map_err(|e| anyhow!("解压lzma时发生错误: {}", e))?;
            Ok(out_buf.into_inner()?)
        }
        [_, "xz"] => {
            xz_decompress(&mut input_buf, &mut out_buf)
                .map_err(|e| anyhow!("解压xz时发生错误: {}", e))?;
            Ok(out_buf.into_inner()?)
        }
        _ => Err(anyhow!("非法文件名: {}", filename)),
    }
}

pub async fn download(
    state: &AppState,
    client: Client,
    url: &str,
    file_name: &str,
    tweak_name: &str,
    bundle_id: &str,
    checksum: MyHash,
    rand_time: u64,
) -> crate::Result<()> {
    tokio::time::sleep(Duration::from_millis(rand_time)).await;
    info!("{}: 随机延时结束", tweak_name);

    info!("{}: 等待获得锁", tweak_name);
    let _guard = state
        .semaphore
        .acquire()
        .await
        .map_err(|e| anyhow!("{}: 无法获得锁: {}", tweak_name, e))?;
    info!("{}: 已获得锁", tweak_name);

    let save_path = state.save_root.join(file_name);
    if save_path.exists() {
        let local_save_path = save_path.clone();
        if tokio::task::spawn_blocking(move || checksum.validate(&local_save_path))
            .await?
            .map_err(|e| anyhow!("计算校验和时发生失败: {}", e))?
        {
            info!(
                " {}, id: {}, url: {}, filename: {} 中 校验和不变，已忽略",
                tweak_name, bundle_id, url, file_name
            );
            return Ok(());
        }
    }
    info!(
        "下载 {}, id: {}, url: {}, filename: {} 中",
        tweak_name, bundle_id, url, file_name
    );
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("下载 {}, 发送请求时发生错误: {}", url, e))?;
    if let Err(e) = tokio::fs::create_dir_all(
        save_path
            .parent()
            .ok_or(anyhow!("{:?} 没有父目录", save_path))?,
    )
    .await
    {
        error!("创建目录失败: {:?},  {}", save_path, e);
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(save_path)
        .await
        .map_err(|e| anyhow!("打开文件时发生错误: {}", e))?;

    while let Some(data) = resp
        .chunk()
        .await
        .map_err(|e| anyhow!("读取块时发生错误: {}", e))?
    {
        file.write(&data[..])
            .await
            .map_err(|e| anyhow!("写块时发生错误: {}", e))?;
    }
    info!("{} 下载完成!", tweak_name);
    return Ok(());
}
