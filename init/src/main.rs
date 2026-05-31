mod disks;

use std::io::{self, Write};
use std::os::unix::fs::symlink;
use nix::mount::{mount, MsFlags};
use std::path::Path;
use std::net::UdpSocket;
use std::time::Duration;
use disks::download_with_retry;
use disks::disk_staff;



fn main() {
    if !Path::new("/proc/self").exists() {
        let _ = mount(
            Some("proc"),
            "/proc",
            Some("proc"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            None::<&str>,
        );
    }
    
    // Монтируем sysfs
    if !Path::new("/sys/kernel").exists() {
        let _ = mount(
            Some("sysfs"),
            "/sys",
            Some("sysfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            None::<&str>,
        );
    }
    
    // Монтируем devtmpfs
    if !Path::new("/dev/zero").exists() {
        let _ = mount(
            Some("devtmpfs"),
            "/dev",
            Some("devtmpfs"),
            MsFlags::MS_NOSUID,
            None::<&str>,
        );
    }
    
    loop {
        println!("\n==================================");
        println!("     MiniOS Installer");
        println!("==================================");
        println!("1) Install MiniOS");
        println!("2) Rescue mode (Busybox)");
        println!("3) Reboot");
        print!("\nChoose [1-3]: ");
        io::stdout().flush().unwrap();

        let mut choice = String::new();
        io::stdin().read_line(&mut choice).unwrap();


        match choice.trim() {
            "1" => {
                if let Err(e) = install() {
                    println!("Installation failed: {}", e);
                    println!("Press Enter to continue...");
                    let mut _buf = String::new();
                    io::stdin().read_line(&mut _buf).unwrap();
                }
            }
            "2" => {
				println!("\n=== Rescue Mode ===");
				println!("Starting BusyBox shell...");
				println!("Type 'exit' to return to installer\n");

				match std::process::Command::new("/bin/sh").status() {
				    Ok(status) => {
				        if !status.success() {
				            println!("Shell exited with error code: {:?}", status.code());
				        }
				    }
				    Err(e) => {
				        println!("Failed to start shell: {}", e);
				    }
				}
            }
            "3" => {
                println!("Rebooting...");
                unsafe { libc::sync(); }
                unsafe { libc::reboot(0x1234567); }
                std::process::exit(0);
            }
            _ => println!("Invalid choice"),
        }
    }
}


fn install() -> Result<(), String> {
println!("\n=== Installation Mode ===");
	let disk = match disks::find_first_disk() {
        Some(disk) => {
            println!("Found HDD: /dev/{} {} ({})",disk.name,disks::format_size(disk.size),disk.model);
            disk
        }
        None => {
            println!("No disks found!");
            println!("Press Enter to continue...");
            let mut _buf = String::new();
            io::stdin().read_line(&mut _buf).unwrap();
            return Err("No disk available".to_string());
		}
	};

	println!("\n=== Partitioning and format disk ===");
	
	let _ = disk_staff(&format!("/dev/{}",disk.name));

	println!("\n=== Mounting root partition ===");
	let mount_point = "/mnt";
	let root_part = format!("/dev/{}1", disk.name);

	std::fs::create_dir_all(mount_point).map_err(|e| format!("Failed to create mount point: {}", e)).unwrap();

	if let Err(e) = nix::mount::mount(Some(root_part.as_str()),mount_point,Some("ext2"),nix::mount::MsFlags::MS_NOATIME,None::<&str>,) {
	    println!("Failed to mount: {}", e);
	}
	println!("✅ Mounted {} to {}", root_part, mount_point);

	let eth0_exists = Path::new("/sys/class/net/eth0").exists();
	if !eth0_exists {
	    println!("ALARM: Eth0  not found !!!");
	} else {
	    println!("✅  Eth0 exists!");
	    let online = match UdpSocket::bind("0.0.0.0:0") {
			Ok(socket) => {
				socket.set_read_timeout(Some(Duration::from_secs(2))).ok();
				let dns_query = [0xaa, 0xaa, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,0x06, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x03, 0x63, 0x6f, 0x6d, 0x00,0x00, 0x01, 0x00, 0x01,];
				match socket.send_to(&dns_query, "1.1.1.1:53") {
					Ok(_) => {
						let mut buf = [0u8; 512];
						socket.recv(&mut buf).is_ok()
				    }
					Err(_) => false,
				}
			}
			Err(_) => false,
			 };

		if online {
			println!("✅ Internet is reachable");
		} else {
			println!("⚠️  No internet connection, but continuing anyway...");
		}
	}

	println!("\n=== Downloading rootfs ===");

	let url = "https://github.com/Mini-OS/Mini-OS/raw/refs/heads/main/rootfs/rootfs.tar.gz";
	let output_path = "/install/rootfs.tar.gz";
	let expected_sha256 = "3538750688079ab5144645e81a93652c1ed4b9572dc3fd8669ccac8cb080f987";

	download_with_retry(url, output_path, 5, expected_sha256)?;

	println!("✅ Download completed");

	println!("\n=== Extracting rootfs ===");

	const DOTS9: &[&str] = &["⢹", "⢺", "⢼", "⣸", "⣇", "⡧", "⡗", "⡏"];
	let mut frame = 0usize;
	let mut last_update = std::time::Instant::now();

	let entries = tar_light::list_entry(output_path)
	    .map_err(|e| format!("Failed to read archive '{}': {}", output_path, e))?;
	let total = entries.len();

	for (i, entry) in entries.iter().enumerate() {
	    let mut name = entry.header.name.clone();
	    while name.starts_with('/') {
	        name = name[1..].to_string();
	    }
	    let stripped = name.trim_start_matches('/');
	    if stripped.is_empty() || stripped.split('/').any(|p| p == "..") {
	        continue;
	    }

	    let dst = Path::new("/mnt").join(stripped);

	    if entry.header.typeflag == b'5' {
	        std::fs::create_dir_all(&dst)
	            .map_err(|e| format!("Failed to create dir {}: {}", dst.display(), e))?;
	    } else if entry.header.typeflag == b'2' {
	        if let Some(parent) = dst.parent() {
	            std::fs::create_dir_all(parent)
	                .map_err(|e| format!("Failed to create parent for {}: {}", dst.display(), e))?;
	        }
	        symlink(&entry.header.linkname, &dst)
	            .map_err(|e| format!("Failed to create symlink {}: {}", dst.display(), e))?;
	    } else {
	        if let Some(parent) = dst.parent() {
	            std::fs::create_dir_all(parent)
	                .map_err(|e| format!("Failed to create parent for {}: {}", dst.display(), e))?;
	        }
	        std::fs::write(&dst, &entry.data)
	            .map_err(|e| format!("Failed to write {}: {}", dst.display(), e))?;
	    }

	    if last_update.elapsed() >= std::time::Duration::from_millis(80) {
	        let pct = (i + 1) * 100 / total;
	        let filled = (pct * 25 / 100).min(25) as usize;
	        let bar: String = (0..25).map(|j| if j < filled { '=' } else { ' ' }).collect();
	        eprint!("\r{} Extracting rootfs... [{}] {}%",
	            DOTS9[frame % DOTS9.len()], bar, pct);
	        frame = (frame + 1) % DOTS9.len();
	        last_update = std::time::Instant::now();
	    }
	}

	eprintln!("\r\x1b[32m✓\x1b[0m Extracting rootfs... Done");
	println!(" Extracted to /mnt");

	println!("\n=== Cleaning up ===");
	std::fs::remove_dir_all("/install").map_err(|e| format!("Failed to remove /install: {}", e))?;
	println!("✅ Removed /install");

	println!("\n=== Configuring fstab ===");
	let fstab_content = format!("/dev/{}1 / ext2 defaults 0 1\n/dev/{}2 none swap sw 0 0\nproc /proc proc defaults 0 0\n",disk.name, disk.name);
	std::fs::write("/mnt/etc/fstab", fstab_content).map_err(|e| format!("Failed to write fstab: {}", e))?;
	println!("✅ fstab created");

	Ok(())
	}
