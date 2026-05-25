mod disks;

use std::io::{self, Write};
use nix::mount::{mount, MsFlags};
use std::path::Path;
use std::net::UdpSocket;
use std::time::Duration;
use disks::download_with_retry;
use disks::format_ext2;
use std::fs;

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
        println!("2) Reboot");
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

	println!("\n=== Partitioning disk ===");

	let sector_size = 512u64;
	let total_sectors = disk.size / sector_size;
	let root_start = 2048u32;

	let mut swap_sectors = (total_sectors / 5) as u32;
	let root_sectors = (total_sectors as u32) - root_start - swap_sectors;
	let swap_start = root_start + root_sectors;
	let swap_last = swap_start + swap_sectors - 1;
	if swap_last > total_sectors as u32 - 1 {
		swap_sectors = (total_sectors as u32) - swap_start;
	}
	println!("Total sectors: {}", total_sectors);
	println!("Root: start={}, sectors={}, end={}", root_start, root_sectors, root_start + root_sectors - 1);
	println!("Swap: start={}, sectors={}, end={}", swap_start, swap_sectors, swap_start + swap_sectors - 1);

	if let Err(e) = disks::create_mbr(
	    &format!("/dev/{}", disk.name),root_start,root_sectors,swap_start,swap_sectors,) {
			println!("MBR creation failed: {}", e);
	}
	if let Err(e) = disks::force_create_partition_nodes(&disk.name, 1) {
	    println!("Warning: failed to create /dev/{}1: {}", disk.name, e);
	}
	if let Err(e) = disks::force_create_partition_nodes(&disk.name, 2) {
	    println!("Warning: failed to create /dev/{}2: {}", disk.name, e);
	}

	std::thread::sleep(std::time::Duration::from_secs(5));

	if !std::path::Path::new(&format!("/dev/{}1", disk.name)).exists() {
	    println!("ERROR: /dev/{}1 not created!", disk.name);
	}
	if !std::path::Path::new(&format!("/dev/{}2", disk.name)).exists() {
	    println!("ERROR: /dev/{}2 not created!", disk.name);
	}

	println!("✅ Partitions created: /dev/{}1 and /dev/{}2", disk.name, disk.name);

	println!("\n=== Formatting partitions ===");
	format_ext2(&format!("/dev/{}1", disk.name)).map_err(|e| format!("Formating  failed: {}", e))?;
	if let Err(e) = disks::create_swap(&format!("/dev/{}2", disk.name)) {
	    println!("Failed to create swap: {}", e);
	}
	unsafe { libc::sync(); }
	std::thread::sleep(std::time::Duration::from_secs(3));

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
	    println!("Eth0 exists!");
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
	std::fs::create_dir_all("/install").map_err(|e| format!("Failed to create /install: {}", e))?;

	download_with_retry(url, output_path, 5)?;
	println!("✅ Download completed");

	println!("\n=== Extracting rootfs ===");
	let tar_gz = std::fs::File::open(output_path).map_err(|e| format!("Failed to open {}: {}", output_path, e))?;

	let tar = flate2::read::GzDecoder::new(tar_gz);
	let mut archive = tar::Archive::new(tar);

	archive.unpack("/mnt").map_err(|e| format!("Failed to extract: {}", e))?;

	println!("✅ Extracted to /mnt");

	println!("\n=== Cleaning up ===");
	std::fs::remove_dir_all("/install").map_err(|e| format!("Failed to remove /install: {}", e))?;
	println!("✅ Removed /install");

	println!("\n=== Configuring fstab ===");
	let fstab_content = format!("/dev/{}1 / ext2 defaults 0 1\n/dev/{}2 none swap sw 0 0\nproc /proc proc defaults 0 0\n",disk.name, disk.name);
	std::fs::write("/mnt/etc/fstab", fstab_content).map_err(|e| format!("Failed to write fstab: {}", e))?;
	println!("✅ fstab created");

// ============================================================
// Монтирование CD-ROM
// ============================================================
println!("\n=== Mounting CD-ROM ===");

let cdrom_dev = "/dev/sr0";
let mount_point = "/cdrom";

match nix::mount::mount(
    Some(cdrom_dev),
    mount_point,
    Some("iso9660"),
    nix::mount::MsFlags::MS_RDONLY,
    None::<&str>,
) {
    Ok(_) => println!("✅ CD-ROM mounted to {}", mount_point),
    Err(e) => {
        return Err(format!("Failed to mount CD-ROM: {}", e));
    }
}

// После распаковки rootfs и настройки fstab
	println!("\n=== Installing bootloader ===");
	if let Err(e) = install_syslinux("/dev/sda", 1, "/mnt", "/cdrom") {
    	println!("Bootloader installation failed: {}", e);
	    return Err(e);
	}

	Ok(())
}


// ============================================================
// Установка загрузчика syslinux на HDD
// ============================================================

	fn write_extlinux_conf(extlinux_dir: &Path, boot_part: u8) -> Result<(), String> {
	    let conf = format!(
	        "DEFAULT minios\n\
	         TIMEOUT 0\n\
	         PROMPT 0\n\
	         \n\
	         LABEL minios\n\
	         \x20   LINUX /boot/kernel\n\
	         \x20   APPEND root=/dev/sda{} rw console=ttyS0\n\
	         \n\
	         LABEL rescue\n\
	         \x20   LINUX /boot/kernel\n\
	         \x20   APPEND root=/dev/sda{} rw console=ttyS0 init=/bin/sh\n",
	        boot_part, boot_part
	    );
	    fs::write(extlinux_dir.join("extlinux.conf"), &conf)
	        .map_err(|e| format!("Failed to write extlinux.conf: {}", e))?;
	    println!("  extlinux.conf written");
	    Ok(())
	}

	fn install_syslinux(disk_dev: &str, boot_part: u8, target_root: &str, cdrom_path: &str) -> Result<(), String> {
	    let target_boot = Path::new(target_root).join("boot");
	    let target_extlinux = Path::new(target_root).join("extlinux");
	    let bl_dir = Path::new("/bootloader");

	    println!("=== Syslinux Installer ===");
	    println!("target root:   {}", target_root);
	    println!("target device: {}", disk_dev);

	    // 1. Создаём директории
	    fs::create_dir_all(&target_boot)
	        .map_err(|e| format!("Failed to create {}: {}", target_boot.display(), e))?;
	    fs::create_dir_all(&target_extlinux)
	        .map_err(|e| format!("Failed to create {}: {}", target_extlinux.display(), e))?;

	    // 2. Копируем ядро с CD-ROM
	    let kernel_src = Path::new(cdrom_path).join("kernel");
	    let kernel_dst = target_boot.join("kernel");
	    if kernel_src.exists() {
	        fs::copy(&kernel_src, &kernel_dst)
	            .map_err(|e| format!("Failed to copy kernel: {}", e))?;
	        println!("  kernel -> /boot/kernel");
	    } else {
	        println!("⚠️ Kernel not found at {}", kernel_src.display());
	    }

	    // 3. Копируем модули extlinux с CD-ROM
	    let modules = ["ldlinux.c32", "libcom32.c32", "libutil.c32", "menu.c32", "vesamenu.c32"];
	    for module in &modules {
	        let src = Path::new(cdrom_path).join(module);
	        let dst = target_extlinux.join(module);
	        if src.exists() {
	            fs::copy(&src, &dst)
	                .map_err(|e| format!("Failed to copy {}: {}", module, e))?;
	        }
	    }
	    println!("  extlinux modules copied");

	    // 4. Записываем конфигурацию
	    write_extlinux_conf(&target_extlinux, boot_part)?;

		println!("  Installing ldlinux.sys manually...");

		// ldlinux.sys лежит в папке /bootloader в нашем initramfs
		let ldlinux_sys_src = Path::new("/bootloader").join("ldlinux.sys");
		let ldlinux_sys_dst = target_extlinux.join("ldlinux.sys");

		if !ldlinux_sys_src.exists() {
		    return Err(format!("ldlinux.sys not found at {}", ldlinux_sys_src.display()));
		}

		fs::copy(&ldlinux_sys_src, &ldlinux_sys_dst)
		    .map_err(|e| format!("Failed to copy ldlinux.sys to extlinux dir: {}", e))?;

		// Копируем также в корень раздела
		let ldlinux_sys_root = Path::new(target_root).join("ldlinux.sys");
		fs::copy(&ldlinux_sys_src, &ldlinux_sys_root)
		    .map_err(|e| format!("Failed to copy ldlinux.sys to root: {}", e))?;

		// Устанавливаем атрибут "immutable" (если есть chattr)
		let _ = std::process::Command::new("chattr")
		    .args(["+i", &ldlinux_sys_root.to_string_lossy()])
		    .status();

		println!("  ldlinux.sys copied and protected");
		
	    // 6. Устанавливаем MBR (altmbr.bin)
	    let mbr_src = bl_dir.join("mbr").join("altmbr.bin");
	    if !mbr_src.exists() {
	        return Err(format!("altmbr.bin not found at {}", mbr_src.display()));
	    }
	    
	    let mbr_data = fs::read(&mbr_src)
	        .map_err(|e| format!("Failed to read altmbr.bin: {}", e))?;
	    
	    if mbr_data.len() < 438 {
	        return Err("altmbr.bin too short".to_string());
	    }
	    
	    let mut patched = mbr_data.clone();
	    patched[437] = boot_part;  // Патчим байт 437 номером раздела
	    
	    let mut disk = std::fs::OpenOptions::new()
	        .write(true)
	        .open(disk_dev)
	        .map_err(|e| format!("Failed to open {}: {}", disk_dev, e))?;
	    
	    use std::io::Write;
	    disk.write_all(&patched[..440])
	        .map_err(|e| format!("Failed to write MBR: {}", e))?;
	    
	    println!("  altmbr.bin (partition {}) written to {}", boot_part, disk_dev);
	    println!("=== Done ===");
	    Ok(())
	}

	fn manual_copy_ldlinux(extlinux_dir: &Path, target_root: &str, cdrom_path: &str) -> Result<(), String> {


Ok(())
}
