use std::path::{Path, PathBuf};

use chrono::Local;
use percent_encoding::percent_decode_str;
use url::Url;

use crate::model::Category;

/// 人类可读的文件大小（1024 进制）。
pub fn format_size(n: u64) -> String {
    const KB: f64 = 1024.0;
    let b = n as f64;
    if b < KB {
        format!("{n} B")
    } else if b < KB * KB {
        format!("{:.2} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.2} MB", b / (KB * KB))
    } else {
        format!("{:.2} GB", b / (KB * KB * KB))
    }
}

pub fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec == 0 {
        "--".to_string()
    } else {
        format!("{}/s", format_size(bytes_per_sec))
    }
}

pub fn format_eta(remaining: u64, speed: u64) -> String {
    if speed == 0 || remaining == 0 {
        return "--:--".to_string();
    }
    let secs = remaining / speed;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 99 {
        ">99h".to_string()
    } else if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn now_str() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 把任意字符串清洗成合法的 Windows 文件名。
pub fn safe_filename(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let mut out = out.trim().trim_matches('.').trim().to_string();
    // Windows 保留设备名
    let upper = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" | "COM5"
            | "COM6" | "COM7" | "COM8" | "COM9" | "LPT1" | "LPT2" | "LPT3"
            | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    );
    if reserved {
        out = format!("_{out}");
    }
    if out.chars().count() > 180 {
        out = out.chars().take(180).collect();
    }
    if out.trim().is_empty() {
        "download".to_string()
    } else {
        out
    }
}

/// 从 URL 路径中推断文件名（忽略查询参数，做百分号解码）。
pub fn filename_from_url(raw: &str) -> Option<String> {
    let u = Url::parse(raw).ok()?;
    let seg = u.path_segments()?.rfind(|s| !s.is_empty())?.to_string();
    let decoded = percent_decode_str(&seg).decode_utf8_lossy().to_string();
    let name = safe_filename(&decoded);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// 解析 `Content-Disposition` 中的文件名，支持 RFC 5987 的 filename*。
pub fn filename_from_content_disposition(value: &str) -> Option<String> {
    for part in value.split(';').map(str::trim) {
        let lower = part.to_ascii_lowercase();
        if lower.starts_with("filename*=") {
            // UTF-8''%E4%B8%AD...  —— 用原始大小写的值取内容
            let raw = part.get(part.find('=')? + 1..)?.trim().trim_matches('"');
            let encoded = raw.splitn(3, '\'').nth(2).unwrap_or(raw);
            let decoded = percent_decode_str(encoded).decode_utf8_lossy().to_string();
            let name = safe_filename(decoded.rsplit('/').next().unwrap_or(&decoded));
            if !name.is_empty() {
                return Some(name);
            }
        } else if lower.strip_prefix("filename=").is_some() {
            let raw = part.get(part.find('=')? + 1..)?.trim().trim_matches('"');
            let raw = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
            let name = safe_filename(raw);
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// `Content-Range: bytes 0-0/12345` → 12345
pub fn parse_content_range_total(value: &str) -> Option<u64> {
    let total = value.rsplit('/').next()?.trim();
    if total == "*" {
        return None;
    }
    total.parse::<u64>().ok()
}

pub fn parse_content_length(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

/// 从 `Content-Type` 推断一个扩展名（用于文件名没有后缀时补齐）。
pub fn ext_from_content_type(ct: &str) -> Option<&'static str> {
    let ct = ct.split(';').next()?.trim().to_ascii_lowercase();
    let ext = match ct.as_str() {
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/x-7z-compressed" => "7z",
        "application/pdf" => "pdf",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/x-msdownload" | "application/x-msdos-program" => "exe",
        "application/octet-stream" => "bin",
        "application/json" => "json",
        "text/plain" => "txt",
        "text/html" => "html",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/flac" => "flac",
        "video/mp4" => "mp4",
        "video/x-matroska" => "mkv",
        "video/webm" => "webm",
        "video/x-flv" => "flv",
        "video/quicktime" => "mov",
        "video/mp2t" => "ts",
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "audio/mpegurl" => "m3u8",
        _ => return None,
    };
    Some(ext)
}

pub fn is_hls_url(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    let path = lower.split('?').next().unwrap_or(&lower);
    path.ends_with(".m3u8") || path.ends_with(".m3u")
}

pub fn category_from_filename(name: &str) -> Category {
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "ts" | "mpg"
        | "mpeg" | "3gp" | "rmvb" | "vob" => Category::Video,
        "mp3" | "m4a" | "wav" | "flac" | "aac" | "ogg" | "wma" | "ape" | "opus" => Category::Audio,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "iso" | "cab" => Category::Archive,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "epub"
        | "rtf" | "csv" => Category::Document,
        "exe" | "msi" | "dll" | "zipapp" | "apk" | "dmg" | "deb" | "rpm" | "bat" | "ps1" => {
            Category::Program
        }
        _ => Category::Other,
    }
}

/// 目标文件已存在时追加 `(1)`、`(2)`……，避免覆盖用户文件。
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for i in 1..1000u32 {
        let p = dir.join(format!("{stem}({i}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{stem}_new{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1024 * 1024), "1.00 MB");
        assert_eq!(format_eta(120, 60), "00:02");
        assert_eq!(format_eta(10, 0), "--:--");
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_filename("a/b:c?.txt"), "a_b_c_.txt");
        assert_eq!(safe_filename("CON"), "_CON");
        assert_eq!(safe_filename("..."), "download");
    }

    #[test]
    fn url_and_disposition() {
        assert_eq!(
            filename_from_url("https://example.com/d/%E4%B8%AD%E6%96%87.zip?x=1"),
            Some("中文.zip".to_string())
        );
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"a b.bin\""),
            Some("a b.bin".to_string())
        );
        assert_eq!(
            filename_from_content_disposition(
                "attachment; filename*=UTF-8''%E6%8A%A5%E5%91%8A.pdf"
            ),
            Some("报告.pdf".to_string())
        );
    }

    #[test]
    fn content_range() {
        assert_eq!(parse_content_range_total("bytes 0-0/12345"), Some(12345));
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
    }

    #[test]
    fn categories() {
        assert_eq!(category_from_filename("a.MP4"), Category::Video);
        assert_eq!(category_from_filename("x.zip"), Category::Archive);
    }
}
