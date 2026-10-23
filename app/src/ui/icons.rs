//! The symbolic icons the UI names, shipped inside the binary (`data/ui-icons`).
//!
//! The icon theme is the system's, and a sandbox does not always have it: the host may name a
//! theme the Flatpak runtime lacks, and GTK then falls back to `hicolor` alone, so every theme icon
//! shows as a "missing image". The bundled icons are laid out like `hicolor`, so they are found whatever
//! theme is in use; a theme that has an icon still wins.

const RESOURCE_PATH: &str = "/io/github/gdr_aislop/jastrzab/icons";

/// Every icon the code names. `tests` keeps this, the SVGs and the sources in step.
const NAMES: [&str; 12] = [
    "edit-copy-symbolic",
    "find-location-symbolic",
    "go-next-symbolic",
    "help-about-symbolic",
    "list-add-symbolic",
    "location-services-active-symbolic",
    "location-services-disabled-symbolic",
    "network-offline-symbolic",
    "network-transmit-receive-symbolic",
    "open-menu-symbolic",
    "process-stop-symbolic",
    "window-close-symbolic",
];

/// Registers the compiled-in icons. Logs which of them the system's theme lacks, which is the first
/// thing to read when a user reports missing images.
///
/// `GtkApplication` only adds `<resource base path>/icons` to the search path if the resource
/// already existed when it started, which it did not, so the path is added here. The check
/// afterwards guards that it worked.
pub fn register() {
    let Some(display) = gtk::gdk::Display::default() else { return };
    let theme = gtk::IconTheme::for_display(&display);
    let lacking = |theme: &gtk::IconTheme| NAMES.iter().filter(|name| !theme.has_icon(name)).copied().collect::<Vec<_>>();
    let missing_from_theme = lacking(&theme);
    tracing::info!(theme = %theme.theme_name(), ?missing_from_theme, "icon theme");

    if let Err(err) = gio::resources_register_include!("jastrzab.gresource") {
        tracing::warn!(%err, "couldn't register the bundled icons");
        return;
    }
    theme.add_resource_path(RESOURCE_PATH);
    let still_missing = lacking(&theme);
    if !still_missing.is_empty() {
        tracing::warn!(?still_missing, "icons missing even with the bundled ones");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use super::NAMES;

    fn manifest() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn files_under(dir: &Path, extension: &str, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files_under(&path, extension, found);
            } else if path.extension().is_some_and(|e| e == extension) {
                found.push(path);
            }
        }
    }

    /// Every string literal in the sources that looks like a symbolic icon name.
    fn names_in_sources() -> BTreeSet<String> {
        let mut files = Vec::new();
        files_under(&manifest().join("src"), "rs", &mut files);
        files_under(&manifest().join("../crates/jastrzab-core/src"), "rs", &mut files);
        let mut names = BTreeSet::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (index, piece) in text.split('"').enumerate() {
                let is_literal = index % 2 == 1;
                if is_literal && piece.ends_with("-symbolic") && piece.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                    names.insert(piece.to_string());
                }
            }
        }
        names
    }

    fn bundled_svgs() -> BTreeSet<String> {
        let mut files = Vec::new();
        files_under(&manifest().join("data/ui-icons"), "svg", &mut files);
        files.iter().map(|f| f.file_stem().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn every_icon_the_code_names_is_bundled() {
        let bundled = bundled_svgs();
        let missing: Vec<_> = names_in_sources().into_iter().filter(|name| !bundled.contains(name)).collect();
        assert!(missing.is_empty(), "icons named in the code but not in app/data/ui-icons (and the gresource XML): {missing:?}");
    }

    #[test]
    fn the_name_list_the_svgs_and_the_resource_file_agree() {
        let listed: BTreeSet<String> = NAMES.iter().map(|n| n.to_string()).collect();
        assert_eq!(listed, bundled_svgs(), "NAMES in src/ui/icons.rs vs the SVGs in data/ui-icons");
        let xml = std::fs::read_to_string(manifest().join("data/jastrzab.gresource.xml")).unwrap();
        for name in &listed {
            assert!(xml.contains(&format!("/{name}.svg</file>")), "{name} is not in jastrzab.gresource.xml");
        }
    }
}
