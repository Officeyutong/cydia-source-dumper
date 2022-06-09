use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about = "by MikuNotFoundException")]
pub struct Args {
    #[clap(help = "仓库URL")]
    pub repo_url: String,
    #[clap(help = "保存目录")]
    pub save_dir: String,
    #[clap(short, long, help = "调试模式(更多日志)")]
    pub debug: bool,
    #[clap(
        short,
        long,
        help = "Cydia ID",
        default_value = "00000000-0001111222233334"
    )]
    pub cydia_id: String,
    #[clap(short, long, help = "固件版本", default_value = "14.7")]
    pub firmware: String,
    #[clap(
        short,
        long,
        help = "唯一ID",
        default_value = "00000000-0001111222233334"
    )]
    pub unique_id: String,
    #[clap(short, long, help = "硬件型号", default_value = "iPhone11,1")]
    pub machine: String,
    #[clap(short, long, help="下载线程数", default_value_t = num_cpus::get() as u32)]
    pub worker: u32,
    #[clap(long, help = "最大失败任务数", default_value_t = 5)]
    pub max_fail_count: u32,
    #[clap(long, short = 'z', help = "打包结果为zip文件")]
    pub pack: bool,
}
