//! What a skin holds, as text.
//!
//! `fastpotify --inspect-wmp-skin skin.wmz` prints this and leaves. It is
//! how a skin is checked without drawing it: what views it defines, what
//! elements those hold, which of them Fastpotify does not draw yet, and
//! what the skin binds to the player.

use std::path::Path;

use crate::wmp::{SkinDocument, Value, ir::Element};

/// Reads a skin and prints its summary; the process's exit code.
pub fn run(path: &Path) -> i32 {
    match SkinDocument::load(path) {
        Ok(document) => {
            print!("{}", dump(&document));
            0
        }
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            1
        }
    }
}

/// The summary of a read skin.
pub fn dump(document: &SkinDocument) -> String {
    let mut out = String::new();
    out.push_str(&format!("Skin: {}\n", document.name));
    let theme = &document.theme;
    let mut known = Vec::new();
    if let Some(title) = &theme.title {
        known.push(format!("title: {title}"));
    }
    if let Some(author) = &theme.author {
        known.push(format!("author: {author}"));
    }
    if !known.is_empty() {
        out.push_str(&format!("{}\n", known.join(", ")));
    }
    if let Some(copyright) = &theme.copyright {
        out.push_str(&format!("copyright: {copyright}\n"));
    }

    out.push_str(&format!("\nViews: {}\n", document.views.len()));
    for view in &document.views {
        let size = match (view.width, view.height) {
            (Some(width), Some(height)) => format!("{width}x{height}"),
            (Some(width), None) => format!("{width} wide"),
            (None, Some(height)) => format!("{height} tall"),
            (None, None) => "unsized".to_string(),
        };
        let id = view.id.as_deref().unwrap_or("<unnamed>");
        let current = document
            .main_view()
            .is_some_and(|main| main.id == view.id && view.id.is_some());
        out.push_str(&format!(
            "  {id} {size}{}\n",
            if current { " (current)" } else { "" }
        ));
    }

    let (kinds, bindings, actions, others) = tally(document);
    let width = kinds.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    out.push_str("\nElements:\n");
    for (name, count) in &kinds {
        out.push_str(&format!("  {name:<width$} {count:>5}\n"));
    }

    if !document.scripts.is_empty() {
        out.push_str("\nScripts (never executed):\n");
        for script in &document.scripts {
            out.push_str(&format!("  {script}\n"));
        }
    }

    if !others.is_empty() {
        out.push_str("\nUnsupported:\n");
        for (name, count) in &others {
            out.push_str(&format!("  {name:<width$} {count:>5}\n"));
        }
    }

    if !bindings.is_empty() {
        out.push_str("\nBindings:\n");
        let width = bindings
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        for (name, count) in &bindings {
            out.push_str(&format!("  {name:<width$} {count:>5}\n"));
        }
    }

    if !actions.is_empty() {
        out.push_str("\nActions:\n");
        let width = actions
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        for (name, count) in &actions {
            out.push_str(&format!("  {name:<width$} {count:>5}\n"));
        }
    }
    out
}

/// The kinds of element a skin uses, the bindings and actions its
/// controls carry, and the elements Fastpotify does not draw yet. Each
/// list is by count, then by name.
type Tally = (
    Vec<(String, usize)>,
    Vec<(String, usize)>,
    Vec<(String, usize)>,
    Vec<(String, usize)>,
);

fn tally(document: &SkinDocument) -> Tally {
    let mut kinds: Vec<(String, usize)> = Vec::new();
    let mut bindings: Vec<(String, usize)> = Vec::new();
    let mut actions: Vec<(String, usize)> = Vec::new();
    let mut others: Vec<(String, usize)> = Vec::new();
    let mut add = |tally: &mut Vec<(String, usize)>, name: String| {
        if let Some(entry) = tally.iter_mut().find(|(kept, _)| *kept == name) {
            entry.1 += 1;
        } else {
            tally.push((name, 1));
        }
    };
    for view in &document.views {
        for element in view.children.iter().flat_map(Element::walk) {
            match element {
                Element::Subview(_) => add(&mut kinds, "SUBVIEW".to_string()),
                Element::Image(_) => add(&mut kinds, "IMAGE".to_string()),
                Element::Button(button) => {
                    add(&mut kinds, "BUTTON".to_string());
                    record_action(&mut actions, &button.action, &mut add);
                }
                Element::ButtonGroup(group) => {
                    add(&mut kinds, "BUTTONGROUP".to_string());
                    for button in &group.buttons {
                        add(&mut kinds, "BUTTONELEMENT".to_string());
                        record_action(&mut actions, &button.action, &mut add);
                    }
                }
                Element::Slider(slider) => {
                    add(&mut kinds, "SLIDER".to_string());
                    if let Some(binding) = &slider.binding {
                        add(&mut bindings, binding.label().to_string());
                    }
                }
                Element::Text(text) => {
                    add(&mut kinds, "TEXT".to_string());
                    if let Some(binding) = &text.binding {
                        add(&mut bindings, binding.label().to_string());
                    }
                }
                Element::Other(other) => {
                    add(&mut others, other.name.to_ascii_uppercase());
                }
            }
            let common = element.common();
            if let Some(binding) = common.visible.as_ref().and_then(Value::binding) {
                add(&mut bindings, binding.label().to_string());
            }
        }
    }
    let by_count_then_name = |(a_name, a_count): &(String, usize),
                              (b_name, b_count): &(String, usize)| {
        b_count.cmp(a_count).then_with(|| a_name.cmp(b_name))
    };
    for tally in [&mut kinds, &mut bindings, &mut actions, &mut others] {
        tally.sort_by(by_count_then_name);
    }
    (kinds, bindings, actions, others)
}

fn record_action(
    actions: &mut Vec<(String, usize)>,
    action: &crate::wmp::ir::Action,
    add: &mut impl FnMut(&mut Vec<(String, usize)>, String),
) {
    add(actions, action.label().to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SkinDocument {
        let definition = br##"<theme title="Sample" author="Microsoft">
            <view id="vMain" width="100" height="50"
                scriptFile="skin.js;">
                <button onClick="view.minimize();"/>
                <slider value="wmpprop:player.settings.volume"/>
                <text value="wmpprop:player.currentmedia.name"/>
                <effects/>
                <playelement/>
            </view>
        </theme>"##;
        SkinDocument::from_files("Sample", [("SKIN.wms", definition.to_vec())]).unwrap()
    }

    #[test]
    fn the_dump_reports_what_the_skin_holds() {
        let dump = dump(&sample());
        assert!(dump.contains("Skin: Sample"), "{dump}");
        assert!(dump.contains("author: Microsoft"), "{dump}");
        assert!(dump.contains("  vMain 100x50 (current)"), "{dump}");
        assert!(dump.contains("BUTTON"), "{dump}");
        assert!(dump.contains("volume"), "{dump}");
        assert!(dump.contains("track-name"), "{dump}");
        assert!(dump.contains("EFFECTS"), "{dump}");
        assert!(
            dump.contains("Scripts (never executed):\n  skin.js"),
            "{dump}"
        );
        assert!(dump.contains("minimize"), "{dump}");
        assert!(dump.contains("play"), "{dump}");
    }

    #[test]
    fn tallies_sort_by_count_then_name() {
        let dump = dump(&sample());
        let elements = dump
            .split_once("\nElements:\n")
            .unwrap()
            .1
            .lines()
            .take_while(|line| line.starts_with("  "))
            .map(|line| line.trim().split_once(' ').unwrap().0.to_string())
            .collect::<Vec<_>>();
        // The two buttons (one written, one predefined) lead, and the
        // single slider sorts ahead of the single text.
        assert_eq!(elements, ["BUTTON", "SLIDER", "TEXT"]);
    }

    #[test]
    fn an_unreadable_skin_exits_with_an_error() {
        assert_eq!(run(Path::new("/nonexistent/skin.wmz")), 1);
    }
}
