use console::{style, Term};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle, ProgressState};
use std::fmt::Write;
use std::time::Duration;

/// Modern, clean progress display for ADIC updates
pub struct DownloadProgressBar {
    #[allow(dead_code)]
    multi_progress: MultiProgress,
    main_bar: ProgressBar,
    #[allow(dead_code)]
    terminal: Term,
}

impl DownloadProgressBar {
    /// Create a new download progress bar
    #[allow(dead_code)]
    pub fn new(version: &str, total_size: u64) -> Self {
        let terminal = Term::stderr();
        let multi_progress = MultiProgress::new();

        // Create main progress bar
        let main_bar = multi_progress.add(ProgressBar::new(total_size));

        // Set up the clean, borderless style
        let style = ProgressStyle::with_template(
            "\n{msg}\n\n{bar:42} {percent:>6}%\n\n{prefix:.dim}"
        )
        .unwrap()
        .progress_chars("▓▓░")
        .with_key("percent", |state: &ProgressState, w: &mut dyn Write| {
            write!(w, "{:>5.1}", state.fraction() * 100.0).unwrap()
        });

        main_bar.set_style(style);
        main_bar.set_message(format!("Downloading ADIC v{}...", version));

        // Enable steady tick for smooth updates
        main_bar.enable_steady_tick(Duration::from_millis(100));

        Self {
            multi_progress,
            main_bar,
            terminal,
        }
    }

    /// Create a progress bar for chunk downloads
    pub fn new_chunk_progress(version: &str, total_chunks: u32) -> Self {
        let terminal = Term::stderr();
        let multi_progress = MultiProgress::new();

        // Create main progress bar
        let main_bar = multi_progress.add(ProgressBar::new(total_chunks as u64));

        // Ultra-clean style optimized for chunks
        let style = ProgressStyle::with_template(
            "{msg}\n\n{bar:42} {percent:>6}%\n\n{prefix}"
        )
        .unwrap()
        .progress_chars("▓▓░")
        .with_key("percent", |state: &ProgressState, w: &mut dyn Write| {
            write!(w, "{:>5.1}", state.fraction() * 100.0).unwrap()
        });

        main_bar.set_style(style);
        main_bar.set_message(format!("Downloading ADIC v{}...", version));

        Self {
            multi_progress,
            main_bar,
            terminal,
        }
    }

    /// Update download progress
    #[allow(dead_code)]
    pub fn update(&self, current: u64, speed_bytes_per_sec: f64, peers: usize, chunks_done: u32, chunks_total: u32) {
        self.main_bar.set_position(current);

        // Format the info line
        let speed_str = format_bytes_per_second(speed_bytes_per_sec);
        let progress_str = format_bytes(current);
        let total_str = format_bytes(self.main_bar.length().unwrap_or(0));

        // Calculate ETA
        let remaining = self.main_bar.length().unwrap_or(0).saturating_sub(current);
        let eta_secs = if speed_bytes_per_sec > 0.0 {
            (remaining as f64 / speed_bytes_per_sec) as u64
        } else {
            0
        };
        let eta_str = format_duration(eta_secs);

        // Build the info line with bullet separators
        let info_line = format!(
            "{} / {}    •    {}    •    {} remaining",
            progress_str, total_str, speed_str, eta_str
        );

        // Build the stats line
        let stats_line = format!(
            "Peers: {}    •    Chunks: {}/{}    •    Verified: {}",
            peers, chunks_done, chunks_total, chunks_done.saturating_sub(1)
        );

        self.main_bar.set_prefix(format!("{}\n{}",
            style(info_line).dim(),
            style(stats_line).dim()
        ));
    }

    /// Update for chunk-based progress
    pub fn update_chunk(&self, chunks_done: u32, chunks_total: u32, speed_chunks_per_sec: f64, peers: usize) {
        self.main_bar.set_position(chunks_done as u64);

        // Calculate ETA based on chunks
        let remaining = chunks_total.saturating_sub(chunks_done);
        let eta_secs = if speed_chunks_per_sec > 0.0 {
            (remaining as f64 / speed_chunks_per_sec) as u64
        } else {
            0
        };
        let eta_str = format_duration(eta_secs);

        // Build the info lines
        let info_line = format!(
            "Chunks: {}/{}    •    {:.1} chunks/s    •    {} remaining",
            chunks_done, chunks_total, speed_chunks_per_sec, eta_str
        );

        let stats_line = format!(
            "Peers: {}    •    Verified: {}",
            peers, chunks_done.saturating_sub(1).min(chunks_done)
        );

        self.main_bar.set_prefix(format!("{}\n{}",
            style(info_line).dim(),
            style(stats_line).dim()
        ));
    }

    /// Update chunk progress with swarm speed
    #[allow(dead_code)]
    pub fn update_chunk_with_swarm(
        &self,
        chunks_done: u32,
        chunks_total: u32,
        local_speed: f64,
        peers: usize,
        swarm_download_speed: f64,
        swarm_upload_speed: f64,
        swarm_peers: usize,
    ) {
        self.main_bar.set_position(chunks_done as u64);

        // Calculate ETA based on chunks
        let remaining = chunks_total.saturating_sub(chunks_done);
        let eta_secs = if local_speed > 0.0 {
            (remaining as f64 / local_speed) as u64
        } else {
            0
        };
        let eta_str = format_duration(eta_secs);

        // Build the info lines
        let info_line = format!(
            "Chunks: {}/{}    •    {:.1} chunks/s    •    {} remaining",
            chunks_done, chunks_total, local_speed, eta_str
        );

        let stats_line = format!(
            "Peers: {}    •    Verified: {}",
            peers, chunks_done.saturating_sub(1).min(chunks_done)
        );

        // Add swarm statistics line
        let swarm_line = format!(
            "Swarm: ↓ {} / ↑ {} ({} peers distributing)",
            format_bytes_per_second(swarm_download_speed),
            format_bytes_per_second(swarm_upload_speed),
            swarm_peers
        );

        self.main_bar.set_prefix(format!("{}\n{}\n{}",
            style(info_line).dim(),
            style(stats_line).dim(),
            style(swarm_line).cyan().dim()
        ));
    }

    /// Show verification stage
    pub fn start_verification(&self) {
        self.main_bar.set_message("Verifying binary integrity...");
        self.main_bar.set_prefix("");

        // Change to a spinner style for verification
        let spinner_style = ProgressStyle::with_template(
            "\n{msg} {spinner}\n"
        )
        .unwrap()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

        self.main_bar.set_style(spinner_style);
        self.main_bar.enable_steady_tick(Duration::from_millis(80));
    }

    /// Mark download as complete
    pub fn finish_success(&self, version: &str) {
        self.main_bar.finish_with_message(
            format!("{}  Successfully downloaded ADIC v{}",
                style("✓").green().bold(),
                version
            )
        );
    }

    /// Mark download as failed
    pub fn finish_error(&self, error: &str) {
        self.main_bar.abandon_with_message(
            format!("{}  Download failed: {}",
                style("✗").red().bold(),
                error
            )
        );
    }

    /// Clear the progress display
    #[allow(dead_code)]
    pub fn clear(&self) {
        self.main_bar.finish_and_clear();
    }
}

/// Format bytes in a human-readable way
#[allow(dead_code)]
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", size as u64, UNITS[unit_idx])
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

/// Format bytes per second
#[allow(dead_code)]
fn format_bytes_per_second(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1.0 {
        return String::from("--");
    }

    let bytes_str = format_bytes(bytes_per_sec as u64);
    format!("{}/s", bytes_str)
}

/// Format duration in a human-readable way
fn format_duration(seconds: u64) -> String {
    if seconds == 0 {
        return String::from("--");
    }

    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        let minutes = seconds / 60;
        let secs = seconds % 60;
        format!("{:02}:{:02}", minutes, secs)
    } else {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    }
}

/// Simple progress indicator for non-TTY environments
pub struct SimpleProgress {
    version: String,
    last_percent: u32,
}

impl SimpleProgress {
    pub fn new(version: &str) -> Self {
        println!("Downloading ADIC v{}...", version);
        Self {
            version: version.to_string(),
            last_percent: 0,
        }
    }

    pub fn update(&mut self, current: u64, total: u64) {
        let percent = ((current as f64 / total as f64) * 100.0) as u32;

        // Only print on 10% increments to avoid spam
        if percent >= self.last_percent + 10 {
            println!("Download progress: {}%", percent);
            self.last_percent = percent;
        }
    }

    pub fn finish_success(&self) {
        println!("✓ Successfully downloaded ADIC v{}", self.version);
    }

    #[allow(dead_code)]
    pub fn finish_error(&self, error: &str) {
        eprintln!("✗ Download failed: {}", error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(5242880), "5.0 MB");
        assert_eq!(format_bytes(1073741824), "1.0 GB");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "--");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(90), "01:30");
        assert_eq!(format_duration(3661), "01:01:01");
    }

    #[test]
    fn test_format_bytes_per_second() {
        assert_eq!(format_bytes_per_second(0.5), "--");
        assert_eq!(format_bytes_per_second(1024.0), "1.0 KB/s");
        assert_eq!(format_bytes_per_second(1048576.0), "1.0 MB/s");
    }
}