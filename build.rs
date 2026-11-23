use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let kernel: PathBuf = env::vars_os()
        .find_map(|(key, value)| {
            if key.to_string_lossy().starts_with("CARGO_BIN_FILE_") {
                Some(value)
            } else {
                None
            }
        })
        .expect("kernel artifact path was not provided by Cargo")
        .into();

    let target = Path::new("target");
    let dist = Path::new("dist");
    let _ = fs::create_dir_all(target);
    let _ = fs::create_dir_all(dist);

    let bios_image = target.join("micro-os-bios.img");
    bootloader::DiskImageBuilder::new(kernel.clone())
        .create_bios_image(&bios_image)
        .expect("failed to create BIOS disk image");

    let uefi_partition = target.join("micro-os-uefi.img");
    bootloader::DiskImageBuilder::new(kernel)
        .create_uefi_fat_partition(&uefi_partition)
        .expect("failed to create UEFI boot partition");

    let iso = target.join("micro-os.iso");
    create_dual_boot_iso(&bios_image, &uefi_partition, &iso)
        .expect("failed to create dual-boot ISO");

    // Copy artifacts to dist/ and workspace root
    let _ = fs::copy(&iso, dist.join("micro-os.iso"));
    let _ = fs::copy(&iso, Path::new("micro-os.iso"));
    let _ = fs::copy(&bios_image, dist.join("micro-os-bios.img"));

    // Copy to user's optical drive path if it exists
    let user_dl_iso = Path::new("C:\\Users\\tilak\\Downloads\\MicroOS-x86_64.iso");
    if user_dl_iso.exists() || user_dl_iso.parent().map(|p| p.exists()).unwrap_or(false) {
        let _ = fs::copy(&iso, user_dl_iso);
    }

    // Convert raw BIOS image to VirtualBox VDI and VMDK formats
    convert_to_virtualbox_disks(&bios_image, target, dist);
}

fn create_dual_boot_iso(
    bios_image_path: &Path,
    uefi_partition_path: &Path,
    iso: &Path,
) -> std::io::Result<()> {
    const BLOCK_SIZE: usize = 2048;
    const BOOT_CATALOG_BLOCK: usize = 19;
    const ROOT_DIRECTORY_BLOCK: usize = 20;
    const PATH_TABLE_BLOCK: usize = 21;
    const BIOS_BOOT_BLOCK: usize = 22;

    let bios_image = fs::read(bios_image_path)?;
    let uefi_image = fs::read(uefi_partition_path)?;

    let bios_blocks = bios_image.len().div_ceil(BLOCK_SIZE);
    let uefi_boot_block = BIOS_BOOT_BLOCK + bios_blocks;
    let uefi_blocks = uefi_image.len().div_ceil(BLOCK_SIZE);
    let total_image_blocks = uefi_boot_block + uefi_blocks;

    let mut image = vec![0; total_image_blocks * BLOCK_SIZE];

    // Place BIOS boot image
    let bios_start = BIOS_BOOT_BLOCK * BLOCK_SIZE;
    image[bios_start..bios_start + bios_image.len()].copy_from_slice(&bios_image);

    // Place UEFI boot image
    let uefi_start = uefi_boot_block * BLOCK_SIZE;
    image[uefi_start..uefi_start + uefi_image.len()].copy_from_slice(&uefi_image);

    // Block 16: Primary Volume Descriptor
    let primary = 16 * BLOCK_SIZE;
    image[primary] = 1;
    image[primary + 1..primary + 6].copy_from_slice(b"CD001");
    image[primary + 6] = 1;
    copy_padded(&mut image[primary + 8..primary + 40], b"MICRO-OS", b' ');
    copy_padded(
        &mut image[primary + 40..primary + 72],
        b"MICRO_OS_DUAL_BOOT",
        b' ',
    );
    set_both_u32(&mut image, primary + 80, total_image_blocks as u32);
    set_both_u16(&mut image, primary + 120, 1);
    set_both_u16(&mut image, primary + 124, 1);
    set_both_u16(&mut image, primary + 128, BLOCK_SIZE as u16);
    set_both_u32(&mut image, primary + 132, 10);
    write_u32_le(&mut image, primary + 140, PATH_TABLE_BLOCK as u32);
    image[primary + 144..primary + 148].fill(0);
    image[primary + 148..primary + 152].copy_from_slice(&(PATH_TABLE_BLOCK as u32).to_be_bytes());
    image[primary + 152..primary + 156].fill(0);
    image[primary + 156] = 34;
    set_both_u32(&mut image, primary + 158, ROOT_DIRECTORY_BLOCK as u32);
    set_both_u32(&mut image, primary + 166, BLOCK_SIZE as u32);
    image[primary + 190] = 1;
    image[primary + 191] = 0;

    // Block 20: Root directory
    let root = ROOT_DIRECTORY_BLOCK * BLOCK_SIZE;
    directory_record(&mut image, root, ROOT_DIRECTORY_BLOCK as u32, 0);
    directory_record(&mut image, root + 34, ROOT_DIRECTORY_BLOCK as u32, 1);

    // Block 21: Path table
    let path_table = PATH_TABLE_BLOCK * BLOCK_SIZE;
    image[path_table] = 1;
    image[path_table + 2..path_table + 6]
        .copy_from_slice(&(ROOT_DIRECTORY_BLOCK as u32).to_le_bytes());
    image[path_table + 6..path_table + 8].copy_from_slice(&1u16.to_le_bytes());

    // Block 17: Boot Record Volume Descriptor (El Torito)
    let boot_record = 17 * BLOCK_SIZE;
    image[boot_record + 1..boot_record + 6].copy_from_slice(b"CD001");
    image[boot_record + 6] = 1;
    copy_padded(
        &mut image[boot_record + 7..boot_record + 39],
        b"EL TORITO SPECIFICATION",
        b' ',
    );
    write_u32_le(&mut image, boot_record + 71, BOOT_CATALOG_BLOCK as u32);

    // Block 18: Volume Descriptor Set Terminator
    image[18 * BLOCK_SIZE] = 255;
    image[18 * BLOCK_SIZE + 1..18 * BLOCK_SIZE + 6].copy_from_slice(b"CD001");
    image[18 * BLOCK_SIZE + 6] = 1;

    // Block 19: Boot Catalog
    let catalog = BOOT_CATALOG_BLOCK * BLOCK_SIZE;

    // 1. Validation Entry (offset 0)
    image[catalog] = 1; // Header ID
    image[catalog + 1] = 0; // Platform ID = 0 (x86 BIOS)
    copy_padded(
        &mut image[catalog + 4..catalog + 28],
        b"MICRO-OS HYBRID BOOT",
        b' ',
    );
    image[catalog + 30] = 0x55;
    image[catalog + 31] = 0xaa;
    let checksum = (0..16)
        .filter(|index| *index != 14)
        .fold(0u16, |sum, index| {
            sum.wrapping_add(u16::from_le_bytes([
                image[catalog + index * 2],
                image[catalog + index * 2 + 1],
            ]))
        });
    let checksum = checksum.wrapping_neg();
    image[catalog + 28..catalog + 30].copy_from_slice(&checksum.to_le_bytes());

    // 2. Initial / Default Entry for x86 BIOS (offset 32)
    // Hard Disk Emulation (0x04): BIOS assigns virtual INT 13h drive (DL=0x80)
    let default_entry = catalog + 32;
    image[default_entry] = 0x88; // Bootable
    image[default_entry + 1] = 0x04; // Media type: 4 = Hard Disk Emulation
    image[default_entry + 6..default_entry + 8].copy_from_slice(&1u16.to_le_bytes()); // 1 MBR sector
    image[default_entry + 8..default_entry + 12]
        .copy_from_slice(&(BIOS_BOOT_BLOCK as u32).to_le_bytes());

    // 3. Section Header for UEFI (offset 64)
    let section = catalog + 64;
    image[section] = 0x91; // Final Section Header
    image[section + 1] = 0xef; // Platform ID = 0xEF (EFI)
    image[section + 2..section + 4].copy_from_slice(&1u16.to_le_bytes()); // 1 entry
    copy_padded(
        &mut image[section + 4..section + 32],
        b"MICRO-OS UEFI",
        b' ',
    );

    // 4. Section Entry for UEFI (offset 96)
    let uefi_sector_count = ((uefi_image.len() / 512) as u16).to_le_bytes();
    let entry = catalog + 96;
    image[entry] = 0x88; // Bootable
    image[entry + 1] = 0x00; // No Emulation
    image[entry + 6..entry + 8].copy_from_slice(&uefi_sector_count);
    image[entry + 8..entry + 12].copy_from_slice(&(uefi_boot_block as u32).to_le_bytes());

    fs::write(iso, &image)?;
    Ok(())
}

fn convert_to_virtualbox_disks(bios_image: &Path, target: &Path, dist: &Path) {
    let target_vdi = target.join("micro-os.vdi");
    let dist_vdi = dist.join("micro-os.vdi");
    let target_vmdk = target.join("micro-os.vmdk");
    let dist_vmdk = dist.join("micro-os.vmdk");

    let vbox_exe = [
        PathBuf::from("C:\\Program Files\\Oracle\\VirtualBox\\VBoxManage.exe"),
        PathBuf::from("C:\\Program Files (x86)\\Oracle\\VirtualBox\\VBoxManage.exe"),
    ];

    let mut vbox_found: Option<PathBuf> = None;
    for path in &vbox_exe {
        if path.exists() {
            vbox_found = Some(path.clone());
            break;
        }
    }

    if let Some(ref vbox) = vbox_found {
        // Stable UUIDs — must never change once registered in VirtualBox.xml
        // dist/micro-os.vdi  → registered in media registry under this UUID
        let dist_vdi_uuid  = "04677654-43c6-4449-a677-3b64d3888f01";
        // microos.vdi in VM folder → registered as the microos VM hard disk
        let user_vdi_uuid  = "6eb0d14c-ecbb-428d-bdfe-ae1523de5cbe";

        // --- dist/micro-os.vdi (always use fixed UUID) ---
        // Close stale registration silently before deleting (UUID mismatch guard)
        let _ = std::process::Command::new(vbox)
            .args(["closemedium", "disk", dist_vdi.to_str().unwrap()])
            .output();
        let _ = fs::remove_file(&dist_vdi);
        let _ = std::process::Command::new(vbox)
            .args([
                "convertfromraw",
                bios_image.to_str().unwrap(),
                dist_vdi.to_str().unwrap(),
                "--format", "VDI",
                "--uuid", dist_vdi_uuid,
            ])
            .output();

        // --- target/micro-os.vdi (build cache, no registered UUID needed) ---
        let _ = fs::remove_file(&target_vdi);
        let _ = std::process::Command::new(vbox)
            .args([
                "convertfromraw",
                bios_image.to_str().unwrap(),
                target_vdi.to_str().unwrap(),
                "--format", "VDI",
            ])
            .output();

        // --- dist/micro-os.vmdk ---
        let _ = fs::remove_file(&target_vmdk);
        let _ = std::process::Command::new(vbox)
            .args([
                "convertfromraw",
                bios_image.to_str().unwrap(),
                target_vmdk.to_str().unwrap(),
                "--format", "VMDK",
            ])
            .output();
        if target_vmdk.exists() {
            let _ = fs::copy(&target_vmdk, &dist_vmdk);
        }

        // --- VM folder VDI (microos.vdi) with its registered UUID ---
        let user_vm_dir = Path::new("C:\\Users\\tilak\\VirtualBox VMs\\microos");
        if user_vm_dir.exists() {
            let user_vdi = user_vm_dir.join("microos.vdi");
            let _ = std::process::Command::new(vbox)
                .args(["closemedium", "disk", user_vdi.to_str().unwrap()])
                .output();
            let _ = fs::remove_file(&user_vdi);
            let _ = std::process::Command::new(vbox)
                .args([
                    "convertfromraw",
                    bios_image.to_str().unwrap(),
                    user_vdi.to_str().unwrap(),
                    "--format", "VDI",
                    "--uuid", user_vdi_uuid,
                ])
                .output();
        }
    } else {
        // Fallback: create raw VMDK descriptor
        create_vmdk_descriptor(bios_image, &target_vmdk);
        let _ = fs::copy(&target_vmdk, &dist_vmdk);
    }
}

fn create_vmdk_descriptor(raw_image: &Path, vmdk_path: &Path) {
    let size_bytes = fs::metadata(raw_image).map(|m| m.len()).unwrap_or(0);
    let sectors = size_bytes / 512;
    let desc = format!(
        "# Disk DescriptorFile\n\
         version=1\n\
         CID=fffffffe\n\
         parentCID=ffffffff\n\
         createType=\"monolithicFlat\"\n\
         \n\
         # Extent description\n\
         RW {} FLAT \"{}\" 0\n\
         \n\
         # The Disk Data Base\n\
         #DDB\n\
         \n\
         ddb.adapterType = \"ide\"\n\
         ddb.geometry.sectors = \"63\"\n\
         ddb.geometry.heads = \"16\"\n\
         ddb.geometry.cylinders = \"{}\"\n\
         ddb.virtualHWVersion = \"4\"\n",
        sectors,
        raw_image.file_name().unwrap().to_string_lossy(),
        (sectors / (63 * 16)).max(1)
    );
    let _ = fs::write(vmdk_path, desc);
}

fn copy_padded(destination: &mut [u8], source: &[u8], padding: u8) {
    destination.fill(padding);
    let copy_len = source.len().min(destination.len());
    destination[..copy_len].copy_from_slice(&source[..copy_len]);
}

fn directory_record(image: &mut [u8], offset: usize, extent: u32, identifier: u8) {
    image[offset] = 34;
    set_both_u32(image, offset + 2, extent);
    set_both_u32(image, offset + 10, 2048);
    image[offset + 25] = 2;
    set_both_u16(image, offset + 28, 1);
    image[offset + 32] = 1;
    image[offset + 33] = identifier;
}

fn write_u32_le(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn set_both_u16(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    buffer[offset + 2..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn set_both_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    buffer[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
}
