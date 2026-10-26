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

#[cfg(test)]
mod tests {
    use super::ellipsize;

    #[test]
    fn ellipsize_cuenta_caracteres_no_bytes() {
        assert_eq!(ellipsize("año", 3), "año");
        assert_eq!(ellipsize("añoñ", 3), "año…");
        assert_eq!(ellipsize("", 3), "");
    }
}
