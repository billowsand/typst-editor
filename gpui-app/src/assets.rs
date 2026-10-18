//! Asset source: the app's own skin (textures, skeuomorphic SVGs) layered
//! over gpui-kit's bundled icon set.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

macro_rules! embedded {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $path)) as &[u8])),*]
    };
}

static SKIN: &[(&str, &[u8])] = embedded![
    "textures/birch.jpg",
    "textures/walnut.jpg",
    "textures/paper.jpg",
    "skin/paperclip.svg",
    "skin/paperclip_v.svg",
    "skin/ribbon.svg",
    "skin/pin.svg",
    "skin/pin_on.svg",
    "skin/star.svg",
    "skin/star_on.svg",
    "skin/notebook.svg",
    "skin/led_green.svg",
    "skin/led_amber.svg",
    "skin/led_red.svg",
    "skin/g_compose.svg",
    "skin/g_folder.svg",
    "skin/g_disk.svg",
    "skin/g_compile.svg",
    "skin/g_plane.svg",
    "skin/g_sidebar.svg",
    "skin/g_triangle.svg",
    "skin/g_search.svg",
    "skin/g_min.svg",
    "skin/g_max.svg",
    "skin/g_close.svg",
    "skin/g_plus.svg",
    "skin/g_outline.svg",
    "skin/g_files.svg",
    "skin/g_doc.svg",
    "skin/g_file.svg",
    "skin/g_image.svg",
    "skin/g_book.svg",
    "skin/g_braces.svg",
    "skin/g_folder_open.svg",
    "skin/g_chevron_down.svg",
    "skin/g_chevron_up.svg",
];

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = SKIN.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out = gpui_kit::assets::Assets.list(path)?;
        out.extend(
            SKIN.iter()
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(*p)),
        );
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::SKIN;

    fn embedded(path: &str) -> bool {
        SKIN.iter().any(|(p, _)| *p == path)
    }

    /// Names passed as the first string argument to `func(` in `src`.
    fn calls<'a>(src: &'a str, func: &str) -> Vec<&'a str> {
        src.match_indices(&format!("{func}(\""))
            .filter_map(|(i, m)| {
                let rest = &src[i + m.len()..];
                rest.find('"').map(|end| &rest[..end])
            })
            .collect()
    }

    /// Every skin image or glyph the views ask for must be embedded, or it
    /// silently renders as nothing.
    #[test]
    fn every_referenced_skin_asset_is_embedded() {
        let sources = [
            include_str!("app.rs"),
            include_str!("preview.rs"),
            include_str!("skin.rs"),
        ];
        let mut missing = Vec::new();
        for src in sources {
            for name in calls(src, "skin") {
                let path = format!("skin/{name}.svg");
                if !embedded(&path) {
                    missing.push(path);
                }
            }
            for func in ["bar_glyph", "paper_glyph"] {
                for name in calls(src, func) {
                    let path = format!("skin/g_{name}.svg");
                    if !embedded(&path) {
                        missing.push(path);
                    }
                }
            }
            for (i, _) in src.match_indices("\"skin/") {
                let rest = &src[i + 1..];
                let path = &rest[..rest.find('"').unwrap()];
                if path.ends_with(".svg") && !path.contains('{') && !embedded(path) {
                    missing.push(path.to_string());
                }
            }
        }
        assert!(missing.is_empty(), "not embedded: {missing:?}");
    }

    #[test]
    fn every_skin_file_on_disk_is_embedded() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/skin");
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                embedded(&format!("skin/{name}")),
                "skin/{name} not embedded"
            );
        }
    }
}
