// ============================================================
// module: disk
// Работа с дисками: обнаружение, вывод информации
// ============================================================

use nix::mount::{mount, MsFlags};
use std::fs::OpenOptions;
use mbrman::{MBR, MBRPartitionEntry, CHS};
use std::time::Duration;
use std::io::Write;
use std::fs;
use std::os::unix::io::FromRawFd;
use std::process::{Command, Output};
use libc;

const EXT2_BIN: &[u8] = include_bytes!("ext2.bin");


pub fn force_create_partition_nodes(disk: &str, partition: u32) -> Result<(), String> {
    
    let part_path = format!("/dev/{}{}", disk, partition);
    
    // В /sys/block/sda/sda1/dev содержится "major:minor"
    let dev_path = format!("/sys/block/{}/{}{}/dev", disk, disk, partition);
    let dev_str = fs::read_to_string(&dev_path)
        .map_err(|e| format!("Failed to read {}: {}", dev_path, e))?;
    let parts: Vec<&str> = dev_str.trim().split(':').collect();
    let major: u32 = parts[0].parse().map_err(|_| "Invalid major")?;
    let minor: u32 = parts[1].parse().map_err(|_| "Invalid minor")?;
    
    // Создаём узел через mknod
    unsafe {
        libc::mknod(
            part_path.as_ptr() as *const _,
            libc::S_IFBLK | 0o600,
            libc::makedev(major, minor),
        );
    }
    
    Ok(())
}

///  структура, которая представляет диск
#[derive(Debug, Clone)]
pub struct Disk {
    pub name: String,   // например, "sda"
    pub size: u64,      // размер в байтах
    pub model: String,  // модель диска
}

/// Форматирует размер в человеко-читаемый вид
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < 3 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size, UNITS[unit])
}

/// Обнаруживает первый доступный диск в системе
pub fn find_first_disk() -> Option<Disk> {
	if !std::path::Path::new("/dev/sda").exists() {
        let _ = std::process::Command::new("mount")
            .args(["-t", "devtmpfs", "devtmpfs", "/dev"])
            .status();
    }
	if !std::path::Path::new("/sys/block").exists() {
        let _ = mount(
            Some("sysfs"),
            "/sys",
            Some("sysfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            None::<&str>,
        );
    }
    println!("Scanning /sys/block...");
    let paths = match std::fs::read_dir("/sys/block") {
        Ok(paths) => paths,
        Err(e) => {
            println!("Cannot read /sys/block: {}", e);
            return None;
        }
    };

    for entry in paths.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        println!("Found device: {}", name);
        
        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("sr") {
            println!("  Skipping virtual device");
            continue;
        }

        let size_path = format!("/sys/block/{}/size", name);
        println!("  Reading size from: {}", size_path);
        let size_str = std::fs::read_to_string(&size_path).unwrap_or_default();
        let sectors: u64 = size_str.trim().parse().unwrap_or(0);
        let size_bytes = sectors * 512;
        println!("  Sectors: {}, Size: {} bytes", sectors, size_bytes);

        if size_bytes == 0 {
            println!("  Zero size, skipping");
            continue;
        }

        let model_path = format!("/sys/block/{}/device/model", name);
        let model = std::fs::read_to_string(&model_path)
            .unwrap_or_default()
            .trim()
            .to_string();
        println!("  Model: '{}'", model);

        let model = if model.is_empty() { "Unknown".to_string() } else { model };

        println!("✅ Found disk: /dev/{}", name);
        return Some(Disk { name, size: size_bytes, model });
    }

    println!("No disks found.");
    None
}


/// Создаёт MBR с двумя разделами (root и swap)
pub fn create_mbr(
    disk_path: &str,
    root_start: u32,
    root_sectors: u32,
    swap_start: u32,
    swap_sectors: u32,
) -> Result<(), String> {
    // 1. Открываем диск на запись
    let mut file = OpenOptions::new()
        .write(true)
        .open(disk_path)
        .map_err(|e| format!("Failed to open {}: {}", disk_path, e))?;

    // 2. Создаём новую, пустую MBR
    let mut mbr = MBR::new_from(&mut file, 512, [0x00; 4])
        .map_err(|e| format!("Failed to create MBR: {}", e))?;

    // 3. Создаём запись для root-раздела (индекс 1)
    let mut root_entry = MBRPartitionEntry::empty();
    root_entry.boot = 0x80;            // Загрузочный флаг
    root_entry.sys = 0x83;             // Тип: Linux filesystem
    root_entry.starting_lba = root_start;
    root_entry.sectors = root_sectors;
    root_entry.first_chs = CHS::empty();
    root_entry.last_chs = CHS::empty();
    mbr[1] = root_entry;               // <- ВАЖНО: индексация с 1

    // 4. Создаём запись для swap-раздела (индекс 2)
    let mut swap_entry = MBRPartitionEntry::empty();
    swap_entry.boot = 0x00;            // Не загрузочный
    swap_entry.sys = 0x82;             // Тип: Linux swap
    swap_entry.starting_lba = swap_start;
    swap_entry.sectors = swap_sectors;
    swap_entry.first_chs = CHS::empty();
    swap_entry.last_chs = CHS::empty();
    mbr[2] = swap_entry;

    // 5. Записываем MBR обратно на диск
    mbr.write_into(&mut file)
        .map_err(|e| format!("Failed to write MBR: {}", e))?;

    // Синхронизируем изменения с диском
    unsafe { libc::sync(); }
    std::thread::sleep(std::time::Duration::from_secs(1));

    println!("✅ MBR created on {}", disk_path);
    Ok(())
}

    

/// Создаёт swap-раздел (записывает заголовок)
pub fn create_swap(partition_path: &str) -> Result<(), String> {  
    let mut file = OpenOptions::new()
        .write(true)
        .open(partition_path)
        .map_err(|e| format!("Failed to open {}: {}", partition_path, e))?;
    
    // Swap header в последних 10 байтах первого килобайта
    // Смещения: 1024 - 10 = 1014
    let offset = 1014;
    let mut header = vec![0u8; 1024];
    
    // Магическое число "swap" (little-endian)
    let magic = 0x73776170u32;
    header[offset] = (magic & 0xff) as u8;
    header[offset + 1] = ((magic >> 8) & 0xff) as u8;
    header[offset + 2] = ((magic >> 16) & 0xff) as u8;
    header[offset + 3] = ((magic >> 24) & 0xff) as u8;
    
    // Версия 1
    let version = 1u32;
    header[offset + 8] = (version & 0xff) as u8;
    header[offset + 9] = ((version >> 8) & 0xff) as u8;
    
    // Размер страницы 4096 байт
    header[offset + 4] = 0x10;
    header[offset + 5] = 0x00;
    
    file.write_all(&header)
        .map_err(|e| format!("Failed to write swap header: {}", e))?;
    
    println!("✅ Swap created on {}", partition_path);
    Ok(())
}


pub fn download_with_retry(url: &str, output_path: &str, max_attempts: u32) -> Result<(), String> {
    for attempt in 1..=max_attempts {
        println!("Download attempt {}/{}...", attempt, max_attempts);
        
        // Создаём клиент с большими таймаутами
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))      // 2 минуты на весь запрос
            .connect_timeout(Duration::from_secs(30)) // 30 секунд на соединение
            .build()
            .map_err(|e| format!("Failed to build client: {}", e))?;
        
        match client.get(url).send() {
            Ok(response) => {
                if !response.status().is_success() {
                    println!("HTTP error: {}, retrying...", response.status());
                    continue;
                }
                
                match response.bytes() {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(output_path, &bytes) {
                            println!("Write error: {}, retrying...", e);
                            continue;
                        }
                        println!("✅ Downloaded {} bytes on attempt {}", bytes.len(), attempt);
                        return Ok(());
                    }
                    Err(e) => {
                        println!("Read error: {}, retrying...", e);
                    }
                }
            }
            Err(e) => {
                println!("Connection error: {}, retrying...", e);
            }
        }
        
        if attempt < max_attempts {
            println!("Waiting 5 seconds before next attempt...");
            std::thread::sleep(Duration::from_secs(5));
        }
    }
    
    Err(format!("Failed to download after {} attempts", max_attempts))
}


pub fn format_ext2(dev_path: &str) -> Result<(), String> {
    let fd = unsafe { libc::memfd_create(b"ext2\0".as_ptr() as *const i8, 0) };
    if fd < 0 {
        return Err("memfd_create failed".into());
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(EXT2_BIN).map_err(|e| e.to_string())?;

    let path = format!("/proc/self/fd/{}", fd);

    let output: Output = Command::new(&path)
        .arg(dev_path)
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}
