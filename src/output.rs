use std::io::{self, IsTerminal, Write};

/// Check if stdout is a TTY.
pub fn is_tty() -> bool {
    io::stdout().is_terminal()
}

/// Print a success message (green checkmark).
pub fn success(msg: &str) {
    if is_tty() {
        eprintln!("  \x1b[32m✓\x1b[0m {}", msg);
    } else {
        eprintln!("  ✓ {}", msg);
    }
}

/// Print an error message (red X).
pub fn error(msg: &str) {
    if is_tty() {
        eprintln!("  \x1b[31m✗\x1b[0m {}", msg);
    } else {
        eprintln!("  ✗ {}", msg);
    }
}

/// Print a warning message (yellow !).
pub fn warn(msg: &str) {
    if is_tty() {
        eprintln!("  \x1b[33m!\x1b[0m {}", msg);
    } else {
        eprintln!("  ! {}", msg);
    }
}

/// Print an info message (dimmed).
pub fn info(msg: &str) {
    if is_tty() {
        eprintln!("  \x1b[2m{}\x1b[0m", msg);
    } else {
        eprintln!("  {}", msg);
    }
}

/// Print a label: value pair with the label dimmed.
pub fn label_value(label: &str, value: &str) {
    if is_tty() {
        eprintln!("  \x1b[2m{}:\x1b[0m {}", label, value);
    } else {
        eprintln!("  {}: {}", label, value);
    }
}

/// Print a commit hash in yellow.
pub fn hash(h: &str) -> String {
    if is_tty() {
        format!("\x1b[33m{}\x1b[0m", h)
    } else {
        h.to_string()
    }
}

/// Print a branch name in cyan.
pub fn branch(b: &str) -> String {
    if is_tty() {
        format!("\x1b[36m{}\x1b[0m", b)
    } else {
        b.to_string()
    }
}

/// Print an operation type with color.
pub fn op_type(op: &str) -> String {
    if is_tty() {
        match op {
            "save" => format!("\x1b[32msave\x1b[0m"),
            "amend" => format!("\x1b[33mamend\x1b[0m"),
            "undo" => format!("\x1b[31mundu\x1b[0m"),
            "redo" => format!("\x1b[36mredo\x1b[0m"),
            "auto" => format!("\x1b[2mauto\x1b[0m"),
            "run" => format!("\x1b[35mrun\x1b[0m"),
            "init" => format!("\x1b[34minit\x1b[0m"),
            _ => op.to_string(),
        }
    } else {
        op.to_string()
    }
}

/// Print a timestamp in dimmed format.
pub fn time_ago(time_str: &str) -> String {
    // Parse ISO 8601 and compute relative time
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(time_str) {
        let now = chrono::Utc::now();
        let duration = now.signed_duration_since(time);

        let ago = if duration.num_seconds() < 60 {
            format!("{}s ago", duration.num_seconds())
        } else if duration.num_minutes() < 60 {
            format!("{}m ago", duration.num_minutes())
        } else if duration.num_hours() < 24 {
            format!("{}h ago", duration.num_hours())
        } else {
            format!("{}d ago", duration.num_days())
        };

        if is_tty() {
            format!("\x1b[2m{}\x1b[0m", ago)
        } else {
            ago
        }
    } else {
        time_str.to_string()
    }
}

/// Print a section header (currently unused).
pub fn _header(msg: &str) {
    if is_tty() {
        eprintln!("\x1b[1m{}\x1b[0m", msg);
    } else {
        eprintln!("{}", msg);
    }
}

/// Print a blank line to stderr.
pub fn blank() {
    eprintln!();
}

/// Flush stderr (currently unused).
pub fn _flush() {
    io::stderr().flush().ok();
}
