pub mod conn_tabs;
pub mod connection_panel;
pub mod db_info;
pub mod filter_bar;
pub mod icons;
pub mod log_panel;
pub mod modals;
pub mod results_grid;
pub mod server_info;
pub mod settings_panel;
pub mod sidebar;
pub mod sql_editor;
pub mod sql_highlight;
pub mod statusbar;
pub mod tab_bar;
pub mod table_structure;
pub mod theme;
pub mod toasts;

/// Prefijo de los atajos con la tecla de comando: Ctrl en Linux y Windows,
/// ⌘ en macOS (egui la expone como `Modifiers::command`).
pub const CMD: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl+"
};

/// Recorta `s` a `max_chars` caracteres, con «…» si sobraban.
pub fn ellipsize(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Tamaño en bytes con la unidad que mejor lo expresa (B, KB, MB o GB).
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::{ellipsize, format_bytes};

    #[test]
    fn format_bytes_elige_la_unidad() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn ellipsize_cuenta_caracteres_no_bytes() {
        assert_eq!(ellipsize("año", 3), "año");
        assert_eq!(ellipsize("añoñ", 3), "año…");
        assert_eq!(ellipsize("", 3), "");
    }
}
