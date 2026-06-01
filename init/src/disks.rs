use nix::mount::{mount, MsFlags};
use std::time::Duration;
use std::io::{Read, Write};
use std::path::Path;
use reqwest::blocking::Client;
use reqwest::header::USER_AGENT;
use sha2::{Sha256, Digest};
use std::fs::File;
use std::io;
use std::error::Error;

const BLKGETSIZE64: libc::c_int = 0x80081272u32 as libc::c_int;

#[derive(Debug, Clone)]
pub struct Disk {
    pub name: String,   // например, "sda"
    pub size: u64,      // размер в байтах
    pub model: String,  // модель диска
}

/// Building human readable disk size
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

// Looking for first available disk
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


// Donloading with retries

pub fn download_text(url: &str) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
    let resp = client.get(url)
        .header(USER_AGENT, "minios-installer/0.1")
        .send()
        .map_err(|e| format!("Failed to download {}: {}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP error {} downloading {}", resp.status(), url));
    }
    resp.text().map_err(|e| format!("Failed to read response: {}", e))
}

const DOTS9_FRAMES: &[&str] = &["⢹", "⢺", "⢼", "⣸", "⣇", "⡧", "⡗", "⡏"];
const BAR_WIDTH: usize = 25;

fn progress_bar(pct: u64) -> String {
    let filled = (pct * BAR_WIDTH as u64 / 100).min(BAR_WIDTH as u64) as usize;
    let mut bar = String::with_capacity(BAR_WIDTH + 2);
    bar.push('[');
    for i in 0..BAR_WIDTH {
        bar.push(if i < filled { '=' } else { ' ' });
    }
    bar.push(']');
    bar
}

fn spinner_tick(frame: &mut usize) -> &'static str {
    let f = DOTS9_FRAMES[*frame % DOTS9_FRAMES.len()];
    *frame = (*frame + 1) % DOTS9_FRAMES.len();
    f
}

pub fn download_with_retry(url: &str, output_path: &str, max_attempts: u32, expected_sha256: &str) -> Result<(), String> {
    let output_dir = Path::new(output_path).parent().unwrap();
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create directory {}: {}", output_dir.display(), e))?;
    
    for attempt in 1..=max_attempts {
        let mut frame = 0usize;
        eprint!("\r{} Download attempt {}/{}...", spinner_tick(&mut frame), attempt, max_attempts);
        
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
        
        let head_response = client.head(url).send();
        let total_size = match head_response {
            Ok(resp) => resp.content_length().unwrap_or(0),
            Err(_) => 0,
        };
        
        let mut response = client.get(url)
            .header(USER_AGENT, "minios-installer/0.1")
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;
        
        if !response.status().is_success() {
            eprintln!("\r\x1b[31m✗\x1b[0m Download attempt {}/{}... HTTP error {}\x1b[K",
                attempt, max_attempts, response.status());
            println!("Retrying...");
            continue;
        }
        
        let total_size = total_size.max(response.content_length().unwrap_or(0));
        
        let temp_path = format!("{}.tmp", output_path);
        let mut file = File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;
        
        let mut buffer = [0u8; 8192];
        let mut bytes_written = 0u64;
        let mut last_update = std::time::Instant::now();
        
        loop {
            let bytes_read = response.read(&mut buffer)
                .map_err(|e| format!("Failed to read response: {}", e))?;
            if bytes_read == 0 {
                break;
            }
            file.write_all(&buffer[..bytes_read])
                .map_err(|e| format!("Failed to write to file: {}", e))?;
            bytes_written += bytes_read as u64;
            
            if last_update.elapsed() >= std::time::Duration::from_millis(80) {
                let frame_char = spinner_tick(&mut frame);
                if total_size > 0 {
                    let pct = bytes_written * 100 / total_size;
                    eprint!("\r{} Download attempt {}/{}... {} {}% ({} MB)",
                        frame_char, attempt, max_attempts,
                        progress_bar(pct), pct, bytes_written / 1_000_000,
                    );
                } else {
                    eprint!("\r{} Download attempt {}/{}... {} bytes",
                        frame_char, attempt, max_attempts, bytes_written,
                    );
                }
                last_update = std::time::Instant::now();
            }
        }
        
        file.flush().map_err(|e| format!("Failed to flush file: {}", e))?;
        
        let mut hasher = Sha256::new();
        let mut file = File::open(&temp_path)
            .map_err(|e| format!("Failed to open temp file for checksum: {}", e))?;
        let mut buffer = [0u8; 8192];
        loop {
            let bytes_read = file.read(&mut buffer)
                .map_err(|e| format!("Failed to read for checksum: {}", e))?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        
        let actual_hash = format!("{:x}", hasher.finalize());
        
        if actual_hash != expected_sha256 {
            eprintln!("\r\x1b[31m✗\x1b[0m Download attempt {}/{}... Checksum mismatch\x1b[K", attempt, max_attempts);
            println!("Expected {}, got {}", expected_sha256, actual_hash);
            std::fs::remove_file(&temp_path).ok();
            if attempt < max_attempts {
                println!("Retrying...");
                continue;
            } else {
                return Err(format!("Checksum mismatch after {} attempts", max_attempts));
            }
        }
        
        std::fs::rename(&temp_path, output_path)
            .map_err(|e| format!("Failed to rename temp file: {}", e))?;
        
        eprintln!("\r\x1b[32m✓\x1b[0m Download attempt {}/{}... Downloaded\x1b[K", attempt, max_attempts);
        println!("Downloaded {} bytes, checksum OK", bytes_written);
        return Ok(());
    }
    
    Err(format!("Failed to download after {} attempts", max_attempts))
}

pub fn get_disk_size(device: &str) -> Result<u64, String> {
	let dev = Path::new(device).file_name().and_then(|n| n.to_str()).unwrap_or(device);
	let sys_path = format!("/sys/block/{}/size", dev);
	match std::fs::read_to_string(&sys_path) {
		Ok(content) => {
			let sectors: u64 = content.trim().parse().map_err(|e|format!("cannot parse {}:     {}", sys_path, e))?;
			Ok(sectors * 512)
   		}
		Err(_) => {
			let cdev = std::ffi::CString::new(device).map_err(|_|"invalid device path".to_string())?;
			let fd = unsafe { libc::open(cdev.as_ptr(),libc::O_RDONLY)
		};
		if fd < 0 {
			return Err(format!("cannot open {}: {}",device,io::Error::last_os_error()));
		}
		let mut size: u64 = 0; 
		let rc = unsafe {
			libc::ioctl(fd, BLKGETSIZE64, &mut size as *mut u64)
      		};
		unsafe { libc::close(fd) };
		if rc < 0 {
			return Err(format!("cannot get size of {}: {}", device, io::Error::last_os_error()));
		} 
		Ok(size)
	}
}
  }
 
pub fn run_sfdisk_script(device: &str, script: &str) -> Result<(), String> {
	let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } < 0 {
    	return Err(format!("pipe failed: {}", io::Error::last_os_error())); 
    }
    let (rfd, wfd) = (fds[0], fds[1]);
    let bytes = script.as_bytes(); 
	let n = unsafe { libc::write(wfd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
    unsafe { libc::close(wfd) };
	if n as usize != bytes.len() {
		unsafe { libc::close(rfd) };
		return Err(format!("pipe write failed {}",io::Error::last_os_error()));
	}
    let saved_stdin = unsafe { libc::dup(0) };
    unsafe { libc::dup2(rfd, 0) };
    unsafe { libc::close(rfd) };
    let rc = sfdisk_sys::sfdisk(&["sfdisk", device]);
    if saved_stdin >= 0 {
        unsafe { libc::dup2(saved_stdin, 0) };
        unsafe { libc::close(saved_stdin) };
    }
    if rc != 0 {
    		return Err(format!("sfdisk failed with code {}", rc));
    }
	Ok(())
}


pub fn disk_staff(device: &str) -> Result<(), Box<dyn Error>> {
    let disk_size = get_disk_size(device)?;
    let disk_mib = disk_size / (1 << 20);
    let available_mib = disk_mib.saturating_sub(1);
    let swap_mib = (available_mib / 4).min(1024);
    let primary_mib = available_mib.saturating_sub(swap_mib);

    if primary_mib < 8 {
        return Err(format!("disk too small ({} MiB)", disk_mib).into());
    }

    eprintln!("disk: {} MiB, primary: {} MiB, swap: {} MiB", disk_mib, primary_mib, swap_mib);

    let script = format!(
    "label: dos\n, {}M, L\n, {}M, S\nwrite\nquit\n", primary_mib, swap_mib);
    run_sfdisk_script(device, &script)?;

    let part1 = format!("{}1", device);
    eprintln!("formatting {} as ext2 (MINI-OS)...", part1);
    ext234_rs::format_fs(&part1, "ext2", "MINI-OS")?;

    if swap_mib > 0 {
        let part2 = format!("{}2", device);
        eprintln!("creating swap on {}...", part2);
        let cfg = swap_rs::MkswapConfig {
            device: part2,
            force: true,
            ..Default::default()
        };
        swap_rs::mkswap(&cfg)?;
    }

    eprintln!("done.");
    Ok(())
}

