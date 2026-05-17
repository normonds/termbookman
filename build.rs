use chrono::Utc;

fn main() {
    let build_time = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    println!("cargo:rustc-env=BUILD_DATE_TIME={}", build_time);
}
