use std::fs::File;
use std::io::Write;
use std::io::Read;
use std::error::Error;
use std::thread;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering}; // 添加 AtomicU64 和 Ordering
// use ctrlc; // 确保在 Cargo.toml 中添加 ctrlc 依赖项

// 全局变量，记录上一次的 CPU 统计值
static PREV_USER: AtomicU64 = AtomicU64::new(0);
static PREV_NICE: AtomicU64 = AtomicU64::new(0);
static PREV_SYSTEM: AtomicU64 = AtomicU64::new(0);
static PREV_IDLE: AtomicU64 = AtomicU64::new(0);

// // 检查 CPU 是否忙碌
// fn is_cpu_busy() -> Result<bool, Box<dyn Error>> {
//     // 打开 /proc/stat 文件
//     let mut file = File::open("/proc/stat")?;
//     let mut contents = String::new();
//     file.read_to_string(&mut contents)?;

//     // 获取第一行并解析
//     let first_line = contents.lines().next().ok_or("Failed to read first line")?;
//     let mut parts = first_line.split_whitespace().skip(1); // 跳过 "cpu"

//     // 提取 user, nice, system, idle 值
//     let user: u64 = parts.next().ok_or("Missing user")?.parse()?;
//     let nice: u64 = parts.next().ok_or("Missing nice")?.parse()?;
//     let system: u64 = parts.next().ok_or("Missing system")?.parse()?;
//     let idle: u64 = parts.next().ok_or("Missing idle")?.parse()?;

//     // 检查 CPU 是否忙碌，并更新上一次的值
//     let busy = user != PREV_USER.load(Ordering::SeqCst) 
//         || nice != PREV_NICE.load(Ordering::SeqCst) 
//         || system != PREV_SYSTEM.load(Ordering::SeqCst)
//         || idle != PREV_IDLE.load(Ordering::SeqCst); // 添加对 idle 的检查
//     PREV_USER.store(user, Ordering::SeqCst);
//     PREV_NICE.store(nice, Ordering::SeqCst);
//     PREV_SYSTEM.store(system, Ordering::SeqCst);
//     PREV_IDLE.store(idle, Ordering::SeqCst);
//     Ok(busy)
// }

fn get_cpu_usage() -> Result<f64, Box<dyn Error>> {
    // 打开 /proc/stat 文件
    let mut file = File::open("/proc/stat")?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    // 获取第一行并解析
    let first_line = contents.lines().next().ok_or("Failed to read first line")?;
    let mut parts = first_line.split_whitespace().skip(1); // 跳过 "cpu"

    // 提取 user, nice, system, idle 值
    let user: u64 = parts.next().ok_or("Missing user")?.parse()?;
    let nice: u64 = parts.next().ok_or("Missing nice")?.parse()?;
    let system: u64 = parts.next().ok_or("Missing system")?.parse()?;
    let idle: u64 = parts.next().ok_or("Missing idle")?.parse()?;

    // 计算总时间和空闲时间
    let total = user + nice + system + idle;
    let idle_time = idle;

    // 获取上一次的总时间和空闲时间
    let prev_total = PREV_USER.load(Ordering::SeqCst) + PREV_NICE.load(Ordering::SeqCst) + PREV_SYSTEM.load(Ordering::SeqCst) + PREV_IDLE.load(Ordering::SeqCst);
    let prev_idle = PREV_IDLE.load(Ordering::SeqCst);

    // 更新上一次的值
    PREV_USER.store(user, Ordering::SeqCst);
    PREV_NICE.store(nice, Ordering::SeqCst);
    PREV_SYSTEM.store(system, Ordering::SeqCst);
    PREV_IDLE.store(idle, Ordering::SeqCst);

    // 计算 CPU 使用率
    let delta_total = total - prev_total;
    let delta_idle = idle_time - prev_idle;
    let usage = 100.0 * (delta_total - delta_idle) as f64 / delta_total as f64;

    Ok(usage)
}

// 检查 RAM 占用情况
fn get_ram_usage() -> Result<f64, Box<dyn Error>> {
    let mut file = File::open("/proc/meminfo")?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    let mut total_mem = 0;
    let mut free_mem = 0;

    for line in contents.lines() {
        if line.starts_with("MemTotal:") {
            total_mem = line.split_whitespace().nth(1).ok_or("Failed to read MemTotal")?.parse()?;
        } else if line.starts_with("MemAvailable:") {
            free_mem = line.split_whitespace().nth(1).ok_or("Failed to read MemAvailable")?.parse()?;
        }
    }

    let used_mem = total_mem - free_mem;
    let usage = 100.0 * used_mem as f64 / total_mem as f64;

    Ok(usage)
}

// 检查 mmcblk1 是否有读写操作
fn is_mmcblk1_busy() -> Result<bool, Box<dyn Error>> {
    let mut file = File::open("/proc/diskstats")?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    for line in contents.lines() {
        if line.contains("mmcblk1") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let reads: u64 = parts[3].parse()?;
            let writes: u64 = parts[7].parse()?;
            static PREV_READS: AtomicU64 = AtomicU64::new(0);
            static PREV_WRITES: AtomicU64 = AtomicU64::new(0);

            let busy = reads != PREV_READS.load(Ordering::SeqCst) || writes != PREV_WRITES.load(Ordering::SeqCst);
            PREV_READS.store(reads, Ordering::SeqCst);
            PREV_WRITES.store(writes, Ordering::SeqCst);
            return Ok(busy);
        }
    }
    Ok(false)
}

// 定义检查间隔（毫秒）
const CPU_CHECK_INTERVAL_MS: u64 = 50; // 例如 10ms

fn set_led_brightness(led_name: &str, brightness: u8) -> Result<(), Box<dyn Error>> {
    let path = format!("/sys/class/leds/{}/brightness", led_name);
    let mut file = File::create(path)?;
    if brightness > 0 {
        // 使用 PWM 输出调整亮度
        let pwm_path = format!("/sys/class/leds/{}/trigger", led_name);
        let mut pwm_file = File::create(pwm_path)?;
        pwm_file.write_all(b"timer")?;
        
        let delay_on_path = format!("/sys/class/leds/{}/delay_on", led_name);
        let mut delay_on_file = File::create(delay_on_path)?;
        let delay_on = (brightness as u16 * 10).to_string(); // 使用 u16 以避免溢出
        delay_on_file.write_all(delay_on.as_bytes())?;
        
        let delay_off_path = format!("/sys/class/leds/{}/delay_off", led_name);
        let mut delay_off_file = File::create(delay_off_path)?;
        let delay_off = (1000u16 - brightness as u16 * 10).to_string(); // 使用 u16 以避免溢出
        delay_off_file.write_all(delay_off.as_bytes())?;
    } else {
        // 关闭 LED
        file.write_all(b"0")?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let led_r = Arc::new(Mutex::new("rgb-led-r"));
    let led_g = Arc::new(Mutex::new("rgb-led-g"));
    let led_b = Arc::new(Mutex::new("rgb-led-b"));

    set_led_brightness("rgb-led-r",0)?;
    set_led_brightness("rgb-led-g",0)?;
    set_led_brightness("rgb-led-b",0)?;

    let led_r_clone = Arc::clone(&led_r);
    let led_g_clone = Arc::clone(&led_g);
    let led_b_clone = Arc::clone(&led_b);

    let _handle_r = thread::spawn(move || {
        loop {
            let usage = get_ram_usage().unwrap_or(0.0); // 使用 unwrap_or 处理错误
            let brightness = if usage < 30.0 {
                0
            } else if usage > 95.0 {
                5
            } else {
                ((usage - 50.0) / 45.0 * 4.0) as u8
            };
            set_led_brightness(&led_r_clone.lock().unwrap(), brightness).unwrap_or(());
            thread::sleep(Duration::from_millis(CPU_CHECK_INTERVAL_MS)); // 使用更高效的睡眠机制
        }
    });

    let _handle_g = thread::spawn(move || {
        loop {
            let usage = get_cpu_usage().unwrap_or(0.0); // 使用 unwrap_or 处理错误
            let brightness = if usage < 30.0 {
                0
            } else if usage > 95.0 {
                5
            } else {
                ((usage - 50.0) / 45.0 * 4.0) as u8
            };
            set_led_brightness(&led_g_clone.lock().unwrap(), brightness).unwrap_or(());
            thread::sleep(Duration::from_millis(CPU_CHECK_INTERVAL_MS)); // 使用更高效的睡眠机制
        }
    });

    let _handle_b = thread::spawn(move || {
        loop {
            let busy = is_mmcblk1_busy().unwrap_or(false); // 使用 unwrap_or 处理错误
            let brightness = if busy { 5 } else { 0 };
            set_led_brightness(&led_b_clone.lock().unwrap(), brightness).unwrap_or(());
            thread::sleep(Duration::from_millis(CPU_CHECK_INTERVAL_MS)); // 使用更高效的睡眠机制
        }
    });

    _handle_r.join().unwrap();
    _handle_g.join().unwrap();
    _handle_b.join().unwrap();

    Ok(())
}
