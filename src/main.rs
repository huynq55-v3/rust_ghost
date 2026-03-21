//! Rust Ghost (WinPE Edition) - NTFS-Aware Partition Backup & Restore
//!
//! A CLI tool that creates compressed disk images by reading only used clusters
//! from NTFS partitions, and can restore those images back.
//!
//! Usage:
//!   rust_ghost.exe backup
//!   rust_ghost.exe restore
//!   rust_ghost.exe backup --source D --dest E:\backup.gho
//!   rust_ghost.exe restore --image E:\backup.gho --target D

mod winapi;
mod ntfs_bitmap;
mod image;
mod backup;
mod restore;
mod verify;

use std::io::{self, Write};
use clap::{Parser, Subcommand};

/// Rust Ghost (WinPE Edition) - NTFS Partition Backup & Restore
#[derive(Parser)]
#[command(name = "rust_ghost")]
#[command(about = "Cong cu backup/restore partition NTFS tren WinPE", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Tao image backup tu mot partition NTFS
    Backup {
        /// Ky tu o dia nguon (vi du: D, E, F). Neu khong chi dinh, se hoi nguoi dung.
        #[arg(short, long)]
        source: Option<String>,

        /// Duong dan file image dich (vi du: E:\backup.gho). Neu khong chi dinh, se hoi nguoi dung.
        #[arg(short, long)]
        dest: Option<String>,

        /// Muc do nen zstd (1-22, mac dinh: 3). Level cao hon = nen tot hon nhung cham hon.
        #[arg(short, long, default_value_t = 3)]
        level: i32,
    },
    /// Restore image backup nguoc lai vao partition
    Restore {
        /// Duong dan file image (.gho). Neu khong chi dinh, se hoi nguoi dung.
        #[arg(short, long)]
        image: Option<String>,

        /// Ky tu o dia dich (vi du: D, E, F). Neu khong chi dinh, se hoi nguoi dung.
        #[arg(short, long)]
        target: Option<String>,
    },
    /// Kiem tra tinh toan ven cua file image (.gho)
    Verify {
        /// Duong dan file image (.gho). Neu khong chi dinh, se hoi nguoi dung.
        #[arg(short, long)]
        image: Option<String>,
    },
}

fn main() {
    println!();
    println!("╔══════════════════════════════════════════════╗");
    println!("║   🔥 RUST GHOST (WinPE Edition) v0.1.0 🔥  ║");
    println!("║   NTFS-Aware Partition Backup & Restore     ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Backup { source, dest, level } => {
            run_backup(source, dest, level)
        }
        Commands::Restore { image, target } => {
            run_restore(image, target)
        }
        Commands::Verify { image } => {
            run_verify(image)
        }
    };

    if let Err(e) = result {
        eprintln!();
        eprintln!("❌ LOI: {}", e);
        eprintln!();
        std::process::exit(1);
    }
}

fn run_backup(source: Option<String>, dest: Option<String>, level: i32) -> io::Result<()> {
    // List available volumes
    println!("📋 Cac o dia co san:");
    println!("{}", "-".repeat(50));
    match winapi::list_volumes() {
        Ok(volumes) => {
            for vol in &volumes {
                println!(
                    "  [{}:] Tong: {} | Trong: {}",
                    vol.letter,
                    vol.total_display(),
                    vol.free_display(),
                );
            }
        }
        Err(e) => {
            eprintln!("  Khong the liet ke o dia: {}", e);
        }
    }
    println!("{}", "-".repeat(50));
    println!();

    // Get source partition
    let source_letter = match source {
        Some(s) => s.trim().trim_end_matches(':').to_uppercase(),
        None => {
            print!("Nhap ky tu o dia NGUON de backup (vi du: D): ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.trim().trim_end_matches(':').to_uppercase()
        }
    };

    if source_letter.is_empty() || source_letter.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ky tu o dia khong hop le! Chi nhap 1 ky tu (vi du: D)",
        ));
    }

    // Get destination path
    let dest_path = match dest {
        Some(d) => d,
        None => {
            print!("Nhap duong dan file image DICH (vi du: E:\\backup.gho): ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    if dest_path.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Duong dan file dich khong duoc de trong!",
        ));
    }

    // Validate compression level
    if !(1..=22).contains(&level) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Muc nen phai tu 1 den 22!",
        ));
    }

    println!();
    println!("⚙️  Cau hinh:");
    println!("  Nguon:    \\\\.\\{}:", source_letter);
    println!("  Dich:     {}", dest_path);
    println!("  Muc nen:  {} (zstd)", level);
    println!();

    // Confirm
    print!("Ban co chac chan muon bat dau BACKUP? (y/n): ");
    io::stdout().flush()?;
    let mut confirm = String::new();
    io::stdin().read_line(&mut confirm)?;
    if confirm.trim().to_lowercase() != "y" {
        println!("Da huy backup.");
        return Ok(());
    }
    println!();

    backup::create_backup(&source_letter, &dest_path, level)
}

fn run_restore(image: Option<String>, target: Option<String>) -> io::Result<()> {
    // Get image file path
    let image_path = match image {
        Some(i) => i,
        None => {
            print!("Nhap duong dan file image (.gho): ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    if image_path.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Duong dan file image khong duoc de trong!",
        ));
    }

    // List available volumes
    println!();
    println!("📋 Cac o dia co san:");
    println!("{}", "-".repeat(50));
    match winapi::list_volumes() {
        Ok(volumes) => {
            for vol in &volumes {
                println!(
                    "  [{}:] Tong: {} | Trong: {}",
                    vol.letter,
                    vol.total_display(),
                    vol.free_display(),
                );
            }
        }
        Err(e) => {
            eprintln!("  Khong the liet ke o dia: {}", e);
        }
    }
    println!("{}", "-".repeat(50));
    println!();

    // Get target partition
    let target_letter = match target {
        Some(t) => t.trim().trim_end_matches(':').to_uppercase(),
        None => {
            print!("Nhap ky tu o dia DICH de restore (vi du: D): ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.trim().trim_end_matches(':').to_uppercase()
        }
    };

    if target_letter.is_empty() || target_letter.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ky tu o dia khong hop le! Chi nhap 1 ky tu (vi du: D)",
        ));
    }

    println!();
    println!("⚠️  CANH BAO: Restore se GHI DE toan bo du lieu tren o [{}:]!", target_letter);
    println!("⚠️  Tao dung lam bay nha!!!");
    println!();
    print!("Nhap 'YES' (viet hoa) de xac nhan RESTORE: ");
    io::stdout().flush()?;
    let mut confirm = String::new();
    io::stdin().read_line(&mut confirm)?;
    if confirm.trim() != "YES" {
        println!("Da huy restore.");
        return Ok(());
    }
    println!();

    restore::restore_image(&image_path, &target_letter)
}

fn run_verify(image: Option<String>) -> io::Result<()> {
    // Get image file path
    let image_path = match image {
        Some(i) => i,
        None => {
            print!("Nhap duong dan file image (.gho) can verify: ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    if image_path.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Duong dan file image khong duoc de trong!",
        ));
    }

    verify::verify_image(&image_path)
}
