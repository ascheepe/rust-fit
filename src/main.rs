use std::env;
use std::error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

fn usage() {
    println!("usage: fit [--source-directory=dir] [--link-destination=dir]");
    println!("           [--bucket-capacity=capacity] [--recursive]");
    println!("           [--dry-run] [--verbose]");
    std::process::exit(0);
}

struct Config {
    source_directory: PathBuf,
    link_destination: PathBuf,
    bucket_capacity: u64,
    recursive: bool,
    dry_run: bool,
    verbose: bool,
}

struct FileInfo {
    path: PathBuf,
    size: u64,
}

impl fmt::Display for FileInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:-8} {}", HumanNumber(self.size), self.path.display())
    }
}

struct Bucket<'a> {
    path: PathBuf,
    capacity: u64,
    size: u64,
    contents: Vec<&'a FileInfo>,
}

impl<'a> fmt::Display for Bucket<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header = format!(
            "Bucket \"{}\": {}/{} ({}%).",
            self.path.display(),
            HumanNumber(self.size),
            HumanNumber(self.capacity),
            self.size * 100 / self.capacity,
        );
        writeln!(f, "{}", "-".repeat(header.len()))?;
        writeln!(f, "{}", header)?;
        writeln!(f, "{}", "-".repeat(header.len()))?;
        for file in &self.contents {
            writeln!(f, "{}", file)?;
        }

        Ok(())
    }
}

impl<'a> Bucket<'a> {
    fn add(&mut self, file: &'a FileInfo) -> bool {
        if self.size + file.size <= self.capacity {
            self.contents.push(file);
            self.size += file.size;
            return true;
        }

        false
    }

    fn link(self, verbose: bool) -> io::Result<()> {
        for file in self.contents {
            let mut target = self.path.clone();
            target.push(file.path.clone());
            let dir = target.parent().unwrap();

            if let Ok(false) = fs::exists(dir) {
                fs::create_dir_all(dir).expect(format!("Unable to create {}.", dir.display()).as_ref());
            }

            if verbose {
                println!("{} -> {}", file.path.display(), target.display());
            }

            fs::hard_link(&file.path, target)?;
        }
        Ok(())
    }
}

struct HumanNumber(pub u64);

impl FromStr for HumanNumber {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err("Empty string".into());
        }

        let (number, suffix) = s.trim().split_at(s.len() - 1);
        let value: f64 = number.parse().map_err(|_| "Invalid number")?;
        let multiplier = match suffix {
            "k" => 1024.0,
            "K" => 1000.0,
            "m" => 1024.0 * 1024.0,
            "M" => 1000.0 * 1000.0,
            "g" => 1024.0 * 1024.0 * 1024.0,
            "G" => 1000.0 * 1000.0 * 1000.0,
            "" => 1.0,
            _ => return Err(format!("unknown suffix '{}'", suffix)),
        };

        Ok(HumanNumber((value * multiplier) as u64))
    }
}

impl fmt::Display for HumanNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.0 as f64;

        if n >= 1_000_000_000.0 {
            write!(f, "{:.2}G", n / 1_000_000_000.0)
        } else if n >= 1_000_000.0 {
            write!(f, "{:.2}M", n / 1_000_000.0)
        } else if n >= 1_000.0 {
            write!(f, "{:.2}K", n / 1_000.0)
        } else {
            write!(f, "{}", self.0)
        }
    }
}

fn collect_files(
    from: &Path,
    recursive: bool,
    max_size: u64,
    files: &mut Vec<FileInfo>,
) -> io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let meta = entry.metadata()?;

        if meta.is_dir() && recursive {
            collect_files(&entry.path(), recursive, max_size, files)?;
        }

        if meta.is_file() {
            files.push(FileInfo {
                path: entry.path(),
                size: meta.len(),
            });
        }
    }
    Ok(())
}

fn make_config() -> io::Result<Config> {
    let mut source_directory: PathBuf = PathBuf::from(".");
    let mut link_destination: PathBuf = PathBuf::from("part");
    let mut bucket_capacity: u64 = HumanNumber::from_str("15M").unwrap().0;
    let mut recursive: bool = false;
    let mut dry_run: bool = false;
    let mut verbose: bool = false;

    let args: Vec<String> = env::args().collect();
    for arg in args {
        if arg.starts_with("--source-directory=") {
            if let Some(value) = arg.strip_prefix("--source-directory=") {
                if !value.is_empty() {
                    source_directory = value.into();
                }
            }
        } else if arg.starts_with("--link-destination=") {
            if let Some(value) = arg.strip_prefix("--link-destination=") {
                if !value.is_empty() {
                    link_destination = value.into();
                }
            }
        } else if arg.starts_with("--bucket-capacity=") {
            if let Some(value) = arg.strip_prefix("--bucket-capacity=") {
                bucket_capacity = value.parse::<HumanNumber>().unwrap().0;
            }
        } else {
            if arg == "--recursive" {
                recursive = true;
            } else if arg == "--dry-run" {
                dry_run = true;
            } else if arg == "--verbose" {
                verbose = true;
            } else if arg == "--help" || arg == "-h" {
                usage();
            }
        }
    }

    Ok(Config {
        source_directory: source_directory,
        link_destination: link_destination,
        bucket_capacity: bucket_capacity,
        recursive: recursive,
        dry_run: dry_run,
        verbose: verbose,
    })
}

fn numbered_dir_namer(prefix: &str) -> impl FnMut() -> PathBuf {
    let mut count: u64 = 0;

    move || -> PathBuf {
        count += 1;
        PathBuf::from(format!("{prefix}/{count:03}"))
    }
}

fn main() -> Result<(), Box<dyn error::Error>> {
    let cfg = make_config()?;

    let mut files: Vec<FileInfo> = Vec::new();
    collect_files(
        &cfg.source_directory,
        cfg.recursive,
        cfg.bucket_capacity,
        &mut files,
    )?;

    if files.len() < 1 {
        return Err(format!("No files found in {}.", cfg.source_directory.display()).into());
    }

    files.sort_by(|a, b| b.size.cmp(&a.size));
    if files[0].size > cfg.bucket_capacity {
        return Err(format!(
            "Can never fit {} ({}).",
            files[0].path.display(),
            HumanNumber(files[0].size)
        )
        .into());
    }

    let mut buckets: Vec<Bucket> = Vec::new();
    let mut new_bucket_name = numbered_dir_namer(cfg.link_destination.to_str().unwrap());
    for file in &files {
        let mut added = false;

        for bucket in &mut buckets {
            if bucket.add(&file) {
                added = true;
                break;
            }
        }

        if !added {
            buckets.push(Bucket {
                path: new_bucket_name(),
                capacity: cfg.bucket_capacity,
                size: file.size,
                contents: [file].to_vec(),
            });
        }
    }

    for bucket in buckets {
        if cfg.dry_run {
            println!("{}", bucket);
        } else {
            if let Err(e) = bucket.link(cfg.verbose) {
                println!("{}", e);
            }
        }
    }

    Ok(())
}
