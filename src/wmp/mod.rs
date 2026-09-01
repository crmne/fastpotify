//! Windows Media Player skins.
//!
//! A `.wmz` file is a zip of art and scripts with a `.wms` skin
//! definition inside — the same container a classic Winamp skin comes
//! as, read by [`crate::skin::zip`]. The definition is XML-flavoured
//! text describing a tree of windows, layers, and controls, each
//! positioned in the view's own pixels; this module reads one into a
//! [`SkinDocument`] of typed data the renderer can draw and the
//! inspector can report. The scripts a skin carries are recorded and
//! never executed: skins are untrusted files from someone else's
//! archive, and the controls a skin needs work without them.
//!
//! Where the format came from: Microsoft's archived Windows Media
//! Player SDK, and the skins themselves — see
//! `docs/wmp-skin-research.md` for the survey the reader is built on.

pub mod assets;
pub mod inspect;
pub mod ir;
pub mod layout;
pub mod xml;

use std::path::Path;

use egui;
use thiserror::Error;

pub use assets::Assets;
pub use ir::{
    Action, Background, Binding, Button, ButtonElement, ButtonGroup, ButtonStates, Common,
    Direction, Element, FontStyle, Image, Justification, Other, ScrollDirection, Scrolling, Slider,
    Subview, Text, Theme, Value, View,
};

#[derive(Debug, Error)]
pub enum WmpError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("not a zip archive; a WMP skin is a .wmz file or a folder with a .wms in it")]
    NotAnArchive,
    #[error("{0}")]
    Archive(#[from] crate::skin::zip::ZipError),
    #[error("no .wms skin definition was found inside")]
    NoDefinition,
    #[error("the skin definition could not be read: {0}")]
    Malformed(String),
}

/// One skin, read whole: its definition as typed data, its scripts as
/// named-but-ignored, and its files ready to decode.
pub struct SkinDocument {
    /// The file or folder name, for showing which skin is on.
    pub name: String,
    pub theme: Theme,
    /// The views the theme defines, in document order.
    pub views: Vec<View>,
    /// The script files the skin names that are plain files. These are
    /// never read, let alone run.
    pub scripts: Vec<String>,
    pub assets: Assets,
}

/// A skin file in the skins folder: a `.wmz` archive.
pub fn is_skin_file(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wmz"))
}

/// A skin's name without the archive extension, for showing.
pub fn label(name: &str) -> &str {
    name.rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("wmz"))
        .map_or(name, |(stem, _)| stem)
}

/// A skin listed in the skins folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkinChoice {
    /// The file name, which is what the settings store.
    pub name: String,
    pub path: std::path::PathBuf,
}

impl SkinChoice {
    /// The name without its extension, for showing.
    pub fn label(&self) -> &str {
        label(&self.name)
    }
}

/// Lists the skins folder's `.wmz` files, by name.
pub fn list_skins(folder: &std::path::Path) -> Vec<SkinChoice> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut skins: Vec<SkinChoice> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            is_skin_file(&path).then_some(SkinChoice { name, path })
        })
        .collect();
    skins.sort_by_key(|skin| skin.name.to_lowercase());
    skins
}

/// What a loading thread reports back.
pub struct Loaded {
    /// The name the settings hold for this skin.
    pub name: String,
    pub result: Result<SkinDocument, WmpError>,
}

/// The skin the WMP window wears, read on another thread the way the
/// Winamp skin is.
#[derive(Default)]
pub struct WmpState {
    /// The skin on screen, with its drawing state.
    pub skin: Option<WmpSkin>,
    /// The setting the worn skin answers to.
    pub worn: Option<String>,
    loading: Option<std::sync::mpsc::Receiver<Loaded>>,
    /// The skins folder's `.wmz` files, as last listed.
    pub choices: Vec<SkinChoice>,
    choices_listed: Option<std::time::Instant>,
}

/// A worn skin: the definition it draws from, and the caches it draws
/// with. The textures belong to a window's context and go with it.
pub struct WmpSkin {
    pub document: std::sync::Arc<SkinDocument>,
    pub render: crate::ui::wmp::Render,
}

impl WmpState {
    /// Puts a skin on. Textures are remade from it at the next frame.
    pub fn wear(&mut self, name: Option<String>, skin: WmpSkin) {
        self.skin = Some(skin);
        self.worn = name;
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Starts reading a skin from the skins folder, on another thread.
    pub fn load(&mut self, name: String, folder: &std::path::Path, ctx: &egui::Context) {
        let path = folder.join(&name);
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("wmp-skin-loader".into())
            .spawn(move || {
                let result = SkinDocument::load(&path);
                let _ = sender.send(Loaded { name, result });
                ctx.request_repaint();
            });
        if let Err(error) = spawned {
            log::warn!("could not start reading the WMP skin: {error}");
        }
        self.loading = Some(receiver);
    }

    /// What a loading thread has finished with, if anything.
    pub fn poll(&mut self) -> Option<Loaded> {
        let receiver = self.loading.as_ref()?;
        match receiver.try_recv() {
            Ok(loaded) => {
                self.loading = None;
                Some(loaded)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.loading = None;
                None
            }
        }
    }

    /// Lists the folder again when the last listing is stale.
    pub fn refresh_choices(&mut self, folder: &std::path::Path) {
        if self
            .choices_listed
            .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(5))
        {
            self.list_choices(folder);
        }
    }

    /// Lists the folder: the `.wmz` files in it.
    pub fn list_choices(&mut self, folder: &std::path::Path) {
        self.choices_listed = Some(std::time::Instant::now());
        self.choices = list_skins(folder);
    }

    /// The textures are gone with a window's context; the next frame
    /// makes them again.
    pub fn forget_textures(&mut self) {
        if let Some(skin) = self.skin.as_mut() {
            skin.render.forget_textures();
        }
    }
}

impl SkinDocument {
    /// Reads a `.wmz` file or an unpacked skin folder.
    pub fn load(path: &Path) -> Result<Self, WmpError> {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            Self::from_dir(name, path)
        } else {
            Self::from_archive(name, &std::fs::read(path)?)
        }
    }

    /// Reads a `.wmz` archive. File names are matched without regard to
    /// case or folder, as the player matched them.
    pub fn from_archive(name: impl Into<String>, bytes: &[u8]) -> Result<Self, WmpError> {
        let name = name.into();
        let archive = crate::skin::zip::Archive::parse(bytes).map_err(|error| match error {
            crate::skin::zip::ZipError::NotAnArchive => WmpError::NotAnArchive,
            other => WmpError::Archive(other),
        })?;
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for entry in archive.entries() {
            if entry.is_dir() {
                continue;
            }
            match archive.read(entry) {
                Ok(bytes) => files.push((entry.file_name(), bytes)),
                Err(error) => log::warn!("skin {name}: {error}"),
            }
        }
        Self::from_files(name, files)
    }

    /// Reads an unpacked skin: a folder with the definition in it.
    pub fn from_dir(name: impl Into<String>, dir: &Path) -> Result<Self, WmpError> {
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                files.push((
                    entry.file_name().to_string_lossy().to_ascii_lowercase(),
                    std::fs::read(entry.path())?,
                ));
            }
        }
        Self::from_files(name, files)
    }

    /// Assembles the document from the skin's files. The definition is
    /// the `.wms` the skin named best — the one sharing its name — or
    /// failing that, the first of them.
    pub fn from_files(
        name: impl Into<String>,
        files: impl IntoIterator<Item = (impl AsRef<str>, Vec<u8>)>,
    ) -> Result<Self, WmpError> {
        let name = name.into();
        let files: Vec<(String, Vec<u8>)> = files
            .into_iter()
            .map(|(file, bytes)| (file.as_ref().to_string(), bytes))
            .collect();
        let mut definitions: Vec<(String, Vec<u8>)> = files
            .iter()
            .filter(|(file, _)| file.ends_with(".wms"))
            .cloned()
            .collect();
        definitions.sort_by_key(|(file, _)| {
            let stem = file.trim_end_matches(".wms");
            // The skin's own name sorts first.
            (!stem.eq_ignore_ascii_case(&name), file.clone())
        });
        let Some((_, definition)) = definitions.first() else {
            return Err(WmpError::NoDefinition);
        };
        let nodes =
            xml::parse(definition).map_err(|error| WmpError::Malformed(error.to_string()))?;
        let (theme, views) = ir::theme(&nodes).map_err(WmpError::Malformed)?;
        let scripts: Vec<String> = views
            .iter()
            .flat_map(|view| view.script_files.iter())
            .filter(|file| !file.contains(':'))
            .map(|file| file.to_string())
            .collect();
        Ok(Self {
            name,
            theme,
            views,
            scripts,
            assets: Assets::from_files(files),
        })
    }

    /// The view the player would show first: the one the theme names,
    /// or else the first defined.
    pub fn main_view(&self) -> Option<&View> {
        let id = self.theme.current_view_id.as_deref();
        self.views
            .iter()
            .find(|view| id.is_some_and(|id| view.id.as_deref() == Some(id)))
            .or_else(|| self.views.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small skin in the shape real ones come: definition, art, script.
    fn sample_skin() -> Vec<u8> {
        let definition = br##"<theme author="Microsoft" title="Sample">
            <view id="vMain" width="100" height="50" titleBar="false"
                backgroundImage="base.bmp" transparencyColor="#FF00FF"
                scriptFile="skin.js;">
                <button left="1" top="2" image="b_up.bmp" downImage="b_down.bmp"
                    onClick="view.minimize();"/>
                <text value="wmpprop:player.currentmedia.name"/>
            </view>
        </theme>"##;
        let image = image::RgbImage::from_pixel(100, 50, image::Rgb([10, 20, 30]));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        crate::skin::zip::write(&[
            ("Sample/", b"", false),
            ("Sample/SKIN.WMS", definition, true),
            ("Sample/Base.BMP", &png.into_inner(), true),
            ("Sample/skin.js", b"// not executed", false),
        ])
    }

    #[test]
    fn a_wmz_reads_into_a_document() {
        let document = SkinDocument::from_archive("Sample", &sample_skin()).unwrap();
        assert_eq!(document.name, "Sample");
        assert_eq!(document.theme.title.as_deref(), Some("Sample"));
        assert_eq!(document.scripts, ["skin.js"]);
        let view = document.main_view().unwrap();
        assert_eq!(view.id.as_deref(), Some("vMain"));
        assert_eq!((view.width, view.height), (Some(100), Some(50)));
        assert_eq!(view.background.transparency_color, Some([0xFF, 0, 0xFF]));
        let bitmap = document.assets.bitmap("base.bmp").unwrap();
        assert_eq!((bitmap.width, bitmap.height), (100, 50));
        assert_eq!(bitmap.pixel(0, 0), Some([10, 20, 30, 255]));
        assert!(document.assets.bytes("skin.js").is_some());
    }

    #[test]
    fn the_skin_definition_with_the_skin_s_name_is_chosen() {
        let definition = b"<theme><view width=\"1\" height=\"1\"/></theme>";
        let other = b"<theme><view width=\"2\" height=\"2\"/></theme>";
        let document = SkinDocument::from_files(
            "toothy",
            [
                ("other.wms", other.to_vec()),
                ("readme.txt", b"x".to_vec()),
                ("Toothy.wms", definition.to_vec()),
            ],
        )
        .unwrap();
        assert_eq!(document.main_view().unwrap().width, Some(1));
    }

    #[test]
    fn a_folder_of_files_is_a_skin_too() {
        let dir = std::env::temp_dir().join(format!("fastpotify-wmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("skin.wms"), b"<theme><view/></theme>").unwrap();
        let document = SkinDocument::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(document.views.len(), 1);
    }

    #[test]
    fn what_is_not_a_skin_is_named_as_such() {
        assert!(matches!(
            SkinDocument::from_archive("text", b"just some text"),
            Err(WmpError::NotAnArchive)
        ));
        let archive = crate::skin::zip::write(&[("readme.txt", b"nothing", false)]);
        assert!(matches!(
            SkinDocument::from_archive("empty", &archive),
            Err(WmpError::NoDefinition)
        ));
        // The tolerant reader forgives mis-nested tags, but a definition
        // with nothing it can call a theme is still refused.
        let themeless = crate::skin::zip::write(&[("x.wms", b"<view/>", false)]);
        assert!(matches!(
            SkinDocument::from_archive("themeless", &themeless),
            Err(WmpError::Malformed(_))
        ));
        assert!(matches!(
            SkinDocument::load(Path::new("/nonexistent/skin.wmz")),
            Err(WmpError::Io(_))
        ));
    }

    #[test]
    fn the_main_view_is_the_one_the_theme_names() {
        let document = SkinDocument::from_files(
            "two",
            [(
                "two.wms",
                br#"<theme currentViewID="second">
                    <view id="first" width="1" height="1"/>
                    <view id="second" width="2" height="2"/>
                </theme>"#
                    .to_vec(),
            )],
        )
        .unwrap();
        assert_eq!(document.main_view().unwrap().id.as_deref(), Some("second"));
    }

    /// Loads every skin in `$FASTPOTIFY_WMP_SAMPLES`, when set, to check
    /// the reader against real files without shipping any. Point it at a
    /// folder of `.wmz` files, such as the corpus listed in
    /// `docs/wmp-skin-research.md`.
    #[test]
    fn sample_wmp_skins_load() {
        let Ok(dir) = std::env::var("FASTPOTIFY_WMP_SAMPLES") else {
            return;
        };
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|extension| extension != "wmz") {
                continue;
            }
            let document = SkinDocument::load(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                document.main_view().is_some(),
                "{} defines no view",
                path.display()
            );
            // The inspector walks every view; make sure none of it panics.
            let _ = inspect::dump(&document);
            // The layout arithmetic settles every view's geometry
            // without circling or panicking.
            for view in &document.views {
                let _ = crate::wmp::layout::Layout::build(view);
            }
            seen += 1;
        }
        assert!(seen > 0, "no .wmz files in the samples folder");
    }
}
