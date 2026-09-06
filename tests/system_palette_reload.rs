use fastpotify::theme::Palette;
use std::{fs, process::Command};

#[cfg(not(target_os = "linux"))]
#[test]
fn follow_system_without_a_linux_palette_file_uses_the_built_in_colours() {
    assert_eq!(Palette::from_system(true), Palette::dark());
    assert_eq!(Palette::from_system(false), Palette::light());
}

#[cfg(target_os = "linux")]
#[test]
fn follows_palette_changes_and_falls_back_when_the_file_is_removed() {
    if std::env::var_os("FASTPOTIFY_PALETTE_TEST_CHILD").is_none() {
        let dir = std::env::temp_dir().join(format!("fastpotify-palette-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "follows_palette_changes_and_falls_back_when_the_file_is_removed",
                "--nocapture",
            ])
            .env("FASTPOTIFY_PALETTE_TEST_CHILD", "1")
            .env("XDG_CONFIG_HOME", &dir)
            .env_remove("HOME")
            .status()
            .unwrap();
        fs::remove_dir_all(dir).unwrap();
        assert!(status.success());
        return;
    }
    let path = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap())
        .join("omarchy/current/theme/colors.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let write = |contents: &str| {
        fs::write(&path, contents).unwrap();
        // Distinct mtimes so the cache notices each write.
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    write("background = \"#0d0b1a\"\nforeground = \"#e6e2f5\"\naccent = \"#a97bf0\"\n");
    let dark = Palette::from_system(true);
    assert_eq!(dark.accent, egui::Color32::from_rgb(169, 123, 240));
    assert!(dark.dark);
    write("background = \"#fafafa\"\nforeground = \"#202020\"\naccent = \"#8050c0\"\n");
    let light = Palette::from_system(true);
    assert_eq!(light.accent, egui::Color32::from_rgb(128, 80, 192));
    assert!(!light.dark);
    write("background = invalid\n");
    assert_eq!(
        Palette::from_system(true),
        light,
        "keep last good palette during partial writes"
    );
    fs::remove_file(&path).unwrap();
    assert_eq!(
        Palette::from_system(true),
        Palette::dark(),
        "missing file uses the built-in palette"
    );
}
