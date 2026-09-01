//! The skin definition as typed data.
//!
//! The `.wms` tree becomes a [`Theme`] holding [`View`]s, each holding the
//! elements it is drawn from. Element and attribute names are matched
//! without regard to case, the way Windows Media Player read them, and
//! whatever the player does not know stays in as an [`Element::Other`]
//! with its children, so a skin is never refused for carrying something
//! new.
//!
//! Two attribute conventions change what a value *is* rather than what it
//! says. `wmpprop:` binds an attribute to a player property and is read
//! back as a [`Binding`]; `jscript:` computes an attribute from other
//! elements and is kept as its expression for a later pass. Everything
//! else is a [`Value::Literal`].

use crate::wmp::xml::Node;

pub type Color = [u8; 3];

/// The theme: the skin's own metadata and the views it is drawn from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Theme {
    pub title: Option<String>,
    pub author: Option<String>,
    pub copyright: Option<String>,
    pub version: Option<String>,
    /// The view the player shows first, by its `id`.
    pub current_view_id: Option<String>,
}

/// One window of the skin. A skin may define several and open them as
/// separate windows; Fastpotify renders one at a time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub id: Option<String>,
    pub title: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub background: Background,
    /// Whether the host window frame is shown. Every recognisable skin
    /// turns it off and draws its own.
    pub title_bar: bool,
    pub resizable: bool,
    /// The interval the skin's `ontimer` fires at, in milliseconds.
    pub timer_interval: Option<u32>,
    /// The `scriptFile` entries as written, including any `res://` ones.
    pub script_files: Vec<String>,
    pub children: Vec<Element>,
}

/// A background layer: the colour behind it, the image on it, and the
/// colour keyed out of that image. The keyed pixels are outside the
/// window entirely: not drawn, not clicked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Background {
    pub color: Option<Color>,
    pub image: Option<String>,
    pub transparency_color: Option<Color>,
    pub tiled: bool,
}

/// The attributes every element shares. Left and top are relative to the
/// view, not the parent; skins position everything in absolute pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Common {
    pub id: Option<String>,
    pub left: Option<Value>,
    pub top: Option<Value>,
    pub width: Option<Value>,
    pub height: Option<Value>,
    pub z_index: Option<i32>,
    pub visible: Option<Value>,
    pub enabled: Option<Value>,
    pub tooltip: Option<String>,
    pub clipping_image: Option<String>,
    pub clipping_color: Option<Color>,
    pub alpha_blend: Option<u8>,
}

impl Common {
    /// The element's left edge, when the skin wrote a number.
    pub fn left_i32(&self) -> Option<i32> {
        num(&self.left)
    }
    /// The element's top edge, when the skin wrote a number.
    pub fn top_i32(&self) -> Option<i32> {
        num(&self.top)
    }
    /// The element's width, when the skin wrote a number.
    pub fn width_i32(&self) -> Option<i32> {
        num(&self.width)
    }
    /// The element's height, when the skin wrote a number.
    pub fn height_i32(&self) -> Option<i32> {
        num(&self.height)
    }

    /// The literal truth of `visible`, when it was written as one.
    pub fn visible_bool(&self) -> Option<bool> {
        boolean(&self.visible)
    }

    /// The literal truth of `enabled`, when it was written as one.
    pub fn enabled_bool(&self) -> Option<bool> {
        boolean(&self.enabled)
    }
}

fn num(value: &Option<Value>) -> Option<i32> {
    value.as_ref().and_then(Value::as_i32)
}

fn boolean(value: &Option<Value>) -> Option<bool> {
    value.as_ref().and_then(Value::as_bool)
}

/// One attribute value, as the kind the skin wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Literal(String),
    /// `wmpprop:player.settings.volume` — bound to the player.
    WmpProp(String),
    /// `jscript:treble.left+treble.width` — computed from other elements.
    JScript(String),
}

impl Value {
    /// Sorts a written value into its kind. A trailing semicolon, which
    /// skins carry on computed values, is not part of either payload.
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if let Some(rest) = prefix(raw, "wmpprop:") {
            return Self::WmpProp(statement(rest));
        }
        if let Some(rest) = prefix(raw, "jscript:") {
            return Self::JScript(statement(rest));
        }
        Self::Literal(raw.to_string())
    }

    /// The literal text, when the value was written as one.
    pub fn as_literal(&self) -> Option<&str> {
        match self {
            Self::Literal(text) => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        self.as_literal().and_then(|text| text.trim().parse().ok())
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.as_literal().and_then(|text| text.trim().parse().ok())
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self.as_literal()?.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        }
    }

    /// The player property the value is bound to, when it is one.
    pub fn binding(&self) -> Option<Binding> {
        match self {
            Self::WmpProp(path) => Some(binding_from_path(path)),
            _ => None,
        }
    }
}

/// `prefix` matched without regard to case, the remainder returned.
fn prefix<'a>(raw: &'a str, prefix: &str) -> Option<&'a str> {
    if raw.len() >= prefix.len() && raw[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&raw[prefix.len()..])
    } else {
        None
    }
}

/// The payload of a bound or computed value: outer whitespace and the
/// trailing semicolon skins append are not part of it.
fn statement(rest: &str) -> String {
    rest.trim().trim_end_matches(';').trim().to_string()
}

/// What an element does when used, as far as the definition says. Actions
/// come from the predefined elements, which carry their behaviour with
/// them, and from the handful of `onClick` handlers that name a player
/// verb outright. Everything else is kept as written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    None,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    FastForward,
    Rewind,
    Mute,
    Shuffle,
    Repeat,
    Minimize,
    Close,
    ReturnToMediaCenter,
    OpenView(String),
    CloseView(String),
    ResetEq,
    EffectsNext,
    EffectsPrevious,
    /// A handler Fastpotify does not act on, kept for the record.
    Unhandled(String),
}

impl Action {
    /// A short name, for listing what a skin does.
    pub fn label(&self) -> &str {
        match self {
            Self::None => "none",
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Stop => "stop",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::FastForward => "fast-forward",
            Self::Rewind => "rewind",
            Self::Mute => "mute",
            Self::Shuffle => "shuffle",
            Self::Repeat => "repeat",
            Self::Minimize => "minimize",
            Self::Close => "close",
            Self::ReturnToMediaCenter => "return-to-media-center",
            Self::OpenView(_) => "open-view",
            Self::CloseView(_) => "close-view",
            Self::ResetEq => "reset-eq",
            Self::EffectsNext => "effects-next",
            Self::EffectsPrevious => "effects-previous",
            Self::Unhandled(_) => "unhandled",
        }
    }
}

/// Reads an `onClick` handler. The common verbs of the historical skins
/// are recognised outright; anything else stays as written.
fn action_from_handler(handler: &str) -> Action {
    let line = handler.trim().trim_end_matches(';').trim();
    let call = line.to_ascii_lowercase();
    match call.as_str() {
        "view.minimize()" | "vmain.minimize()" => Action::Minimize,
        "view.close()" | "vmain.close()" => Action::Close,
        "view.returntomediacenter()" => Action::ReturnToMediaCenter,
        "player.controls.play()" => Action::Play,
        "player.controls.pause()" => Action::Pause,
        "player.controls.stop()" => Action::Stop,
        "player.controls.next()" => Action::Next,
        "player.controls.previous()" => Action::Previous,
        "player.settings.mute=down" => Action::Mute,
        "eq.reset()" => Action::ResetEq,
        "viseffects.next()" => Action::EffectsNext,
        "viseffects.previous()" => Action::EffectsPrevious,
        _ if call.starts_with("theme.openview(") => {
            view_argument(line).map_or(Action::None, Action::OpenView)
        }
        _ if call.starts_with("theme.closeview(") => {
            view_argument(line).map_or(Action::None, Action::CloseView)
        }
        _ => Action::Unhandled(handler.trim().to_string()),
    }
}

/// The quoted argument of `theme.openView('id')`.
fn view_argument(line: &str) -> Option<String> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    let argument = line[open + 1..close].trim();
    Some(argument.trim_matches(['\'', '"']).to_string())
}

/// A player property an element shows or drives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    Volume,
    Balance,
    Mute,
    /// Playback position, in seconds, as a number.
    Position,
    /// Playback position as the player writes it, `3:42`.
    PositionString,
    Duration,
    DurationString,
    TrackName,
    SourceUrl,
    DownloadProgress,
    /// One equalizer band's gain, 1 (low) to 10 (high).
    EqGain(u8),
    EqPresetTitle,
    EffectsType,
    EffectsPreset,
    EffectsPresetTitle,
    /// The status line the player writes for itself.
    Status,
    /// A property Fastpotify does not serve, kept for the record.
    Unhandled(String),
}

impl Binding {
    /// A short name, for listing what a skin shows.
    pub fn label(&self) -> &str {
        match self {
            Self::Volume => "volume",
            Self::Balance => "balance",
            Self::Mute => "mute",
            Self::Position => "position",
            Self::PositionString => "position-string",
            Self::Duration => "duration",
            Self::DurationString => "duration-string",
            Self::TrackName => "track-name",
            Self::SourceUrl => "source-url",
            Self::DownloadProgress => "download-progress",
            Self::EqGain(_) => "eq-gain",
            Self::EqPresetTitle => "eq-preset-title",
            Self::EffectsType => "effects-type",
            Self::EffectsPreset => "effects-preset",
            Self::EffectsPresetTitle => "effects-preset-title",
            Self::Status => "status",
            Self::Unhandled(_) => "unhandled",
        }
    }
}

/// Reads a `wmpprop:` path. The properties the historical skins bind are a
/// small closed set; the rest stays as written.
fn binding_from_path(path: &str) -> Binding {
    match path.to_ascii_lowercase().as_str() {
        "player.settings.volume" => Binding::Volume,
        "player.settings.balance" => Binding::Balance,
        "player.settings.mute" => Binding::Mute,
        "player.controls.currentposition" => Binding::Position,
        "player.controls.currentpositionstring" => Binding::PositionString,
        "player.currentmedia.duration" => Binding::Duration,
        "player.currentmedia.durationstring" => Binding::DurationString,
        "player.currentmedia.name" => Binding::TrackName,
        "player.currentmedia.sourceurl" => Binding::SourceUrl,
        "player.network.downloadprogress" => Binding::DownloadProgress,
        "eq.currentpresettitle" => Binding::EqPresetTitle,
        "mediacenter.effecttype" => Binding::EffectsType,
        "mediacenter.effectpreset" => Binding::EffectsPreset,
        "viseffects.currentpresettitle" => Binding::EffectsPresetTitle,
        other => other
            .strip_prefix("eq.gainlevel")
            .and_then(|band| band.parse::<u8>().ok())
            .filter(|band| (1..=10).contains(band))
            .map_or(Binding::Unhandled(path.to_string()), Binding::EqGain),
    }
}

/// The action a predefined element carries by its name alone.
fn predefined_action(name: &str) -> Option<Action> {
    Some(match name {
        "playbutton" | "playelement" => Action::Play,
        "pausebutton" | "pauseelement" => Action::Pause,
        "stopbutton" | "stopelement" => Action::Stop,
        "nextbutton" | "nextelement" => Action::Next,
        "prevbutton" | "prevelement" => Action::Previous,
        "ffwdbutton" | "ffwdelement" => Action::FastForward,
        "rewbutton" | "rewindelement" => Action::Rewind,
        "mutebutton" => Action::Mute,
        "shufflebutton" => Action::Shuffle,
        "repeatbutton" => Action::Repeat,
        "returnbutton" => Action::ReturnToMediaCenter,
        "closebutton" => Action::Close,
        "minimizebutton" => Action::Minimize,
        _ => return None,
    })
}

/// The binding a predefined control carries by its name alone.
fn predefined_binding(name: &str) -> Option<Binding> {
    Some(match name {
        "seekslider" => Binding::Position,
        "volumeslider" => Binding::Volume,
        "balanceslider" => Binding::Balance,
        "currentpositiontext" => Binding::PositionString,
        "durationtext" => Binding::DurationString,
        "statustext" => Binding::Status,
        _ => return None,
    })
}

/// An element of the view: the kinds Fastpotify draws, and everything
/// else collected under [`Element::Other`].
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Subview(Subview),
    Image(Image),
    Button(Button),
    ButtonGroup(ButtonGroup),
    Slider(Slider),
    Text(Text),
    Other(Other),
}

impl Element {
    /// The element's shared attributes, whatever kind it is.
    pub fn common(&self) -> &Common {
        match self {
            Self::Subview(element) => &element.common,
            Self::Image(element) => &element.common,
            Self::Button(element) => &element.common,
            Self::ButtonGroup(element) => &element.common,
            Self::Slider(element) => &element.common,
            Self::Text(element) => &element.common,
            Self::Other(element) => &element.common,
        }
    }

    /// Every element in this one, itself included, in document order.
    pub fn walk(&self) -> Vec<&Element> {
        let mut all = vec![self];
        let children: &[Element] = match self {
            Self::Subview(element) => &element.children,
            Self::Other(element) => &element.children,
            _ => &[],
        };
        for child in children {
            all.extend(child.walk());
        }
        all
    }
}

/// A layer of the view: a background of its own with elements on it.
/// Skins use subviews for drawers, pop-ups, and whole alternate panes,
/// shown and hidden as the player goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subview {
    pub common: Common,
    pub background: Background,
    pub children: Vec<Element>,
}

/// A picture at a position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Image {
    pub common: Common,
    pub image: Option<String>,
    pub transparency_color: Option<Color>,
    pub tiled: bool,
}

/// The images a button or button group wears in each state. Missing
/// states fall back to `image`, the way the player drew them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ButtonStates {
    pub image: Option<String>,
    pub hover: Option<String>,
    pub down: Option<String>,
    pub hover_down: Option<String>,
    pub disabled: Option<String>,
}

/// A button on its own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Button {
    pub common: Common,
    pub states: ButtonStates,
    pub transparency_color: Option<Color>,
    /// A two-state button: it stays down after being clicked.
    pub sticky: bool,
    pub tiled: bool,
    pub action: Action,
}

/// A group of buttons drawn from one bitmap per state and told apart by
/// the colour under the pointer in the mapping bitmap. The transport
/// controls of nearly every skin are a group of predefined elements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ButtonGroup {
    pub common: Common,
    pub states: ButtonStates,
    pub mapping_image: Option<String>,
    pub transparency_color: Option<Color>,
    /// The buttons act like radio buttons: one down at a time.
    pub radio: bool,
    /// Whether the group's own bitmap shows behind the buttons.
    pub show_background: bool,
    pub buttons: Vec<ButtonElement>,
}

/// One button inside a group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ButtonElement {
    pub id: Option<String>,
    /// The colour in the mapping bitmap that is this button.
    pub mapping_color: Option<Color>,
    pub sticky: bool,
    pub tooltip: Option<String>,
    pub enabled: Option<Value>,
    pub action: Action,
}

/// Which way a slider runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    Horizontal,
    Vertical,
}

/// A slider: a track image, a thumb drawn at the value's position, and
/// the player property the value is. Seek sliders bind the position and
/// write it back on drag end; volume sliders bind the volume.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slider {
    pub common: Common,
    pub background_image: Option<String>,
    pub background_hover_image: Option<String>,
    pub foreground_image: Option<String>,
    pub foreground_hover_image: Option<String>,
    pub thumb_image: Option<String>,
    pub thumb_hover_image: Option<String>,
    pub thumb_down_image: Option<String>,
    pub thumb_disabled_image: Option<String>,
    pub direction: Direction,
    /// Slider images tile along the track by default.
    pub tiled: bool,
    /// Pixels of the track image kept at each end, outside the travel.
    pub border_size: i32,
    pub min: Option<Value>,
    pub max: Option<Value>,
    pub value: Option<Value>,
    /// The foreground image shows progress rather than a thumb position.
    pub use_foreground_progress: bool,
    pub foreground_progress: Option<Value>,
    pub transparency_color: Option<Color>,
    pub binding: Option<Binding>,
}

/// How text lines up in its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justification {
    #[default]
    Left,
    Center,
    Right,
}

/// The styles a text may carry, as the skin wrote them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
}

/// Which way scrolling text moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollDirection {
    #[default]
    Left,
    Right,
}

/// A text's scrolling: how far each step goes, how long between steps,
/// and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scrolling {
    pub amount: i32,
    pub delay_ms: u32,
    pub direction: ScrollDirection,
}

/// A piece of text: a system font, a colour, and usually a binding.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Text {
    pub common: Common,
    pub value: Option<Value>,
    pub font_face: Option<String>,
    pub font_size: Option<Value>,
    pub font_style: FontStyle,
    pub foreground_color: Option<Color>,
    pub background_color: Option<Color>,
    pub justification: Justification,
    pub scrolling: Option<Scrolling>,
    pub word_wrap: bool,
    pub binding: Option<Binding>,
}

/// An element Fastpotify does not draw yet, kept with its children so
/// the record of a skin stays complete.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Other {
    /// The element name, lower-cased.
    pub name: String,
    pub common: Common,
    pub children: Vec<Element>,
}

/// Reads the theme and its views out of the parsed skin. The first
/// `theme` element is the skin; a definition without one is an error.
pub fn theme(nodes: &[Node]) -> Result<(Theme, Vec<View>), String> {
    let root = nodes
        .iter()
        .find(|node| node.name == "theme")
        .ok_or_else(|| "no THEME element was found".to_string())?;
    let theme = Theme {
        title: text(root.attr("title")),
        author: text(root.attr("author")),
        copyright: text(root.attr("copyright")),
        version: text(root.attr("version")),
        current_view_id: text(root.attr("currentviewid")),
    };
    let views = root
        .children
        .iter()
        .filter(|node| node.name == "view")
        .map(view)
        .collect();
    Ok((theme, views))
}

fn text(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn color(raw: Option<&str>) -> Option<Color> {
    let digits = raw?.trim().trim_start_matches('#');
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    match digits.len() {
        6 => Some([channel(0)?, channel(2)?, channel(4)?]),
        3 => Some([
            u8::from_str_radix(&digits[0..1], 16).ok()? * 17,
            u8::from_str_radix(&digits[1..2], 16).ok()? * 17,
            u8::from_str_radix(&digits[2..3], 16).ok()? * 17,
        ]),
        _ => None,
    }
}

fn flag(raw: Option<&str>) -> bool {
    raw.and_then(|raw| Value::parse(raw).as_bool())
        .unwrap_or(false)
}

fn i32_(raw: Option<&str>) -> Option<i32> {
    raw.and_then(|raw| Value::parse(raw).as_i32())
}

/// The view's ambient attributes as a [`Common`].
fn common(node: &Node) -> Common {
    Common {
        id: text(node.attr("id")),
        left: node.attr("left").map(Value::parse),
        top: node.attr("top").map(Value::parse),
        width: node.attr("width").map(Value::parse),
        height: node.attr("height").map(Value::parse),
        z_index: i32_(node.attr("zindex")),
        visible: node.attr("visible").map(Value::parse),
        enabled: node.attr("enabled").map(Value::parse),
        tooltip: text(node.attr("tooltip")).or_else(|| text(node.attr("uptooltip"))),
        clipping_image: text(node.attr("clippingimage")),
        clipping_color: color(node.attr("clippingcolor")),
        alpha_blend: node
            .attr("alphablend")
            .and_then(|raw| Value::parse(raw).as_i32())
            .and_then(|value| u8::try_from(value).ok()),
    }
}

fn background(node: &Node) -> Background {
    Background {
        color: color(node.attr("backgroundcolor")),
        image: text(node.attr("backgroundimage")),
        transparency_color: color(node.attr("transparencycolor")),
        tiled: flag(node.attr("backgroundtiled")),
    }
}

fn view(node: &Node) -> View {
    View {
        id: text(node.attr("id")),
        title: text(node.attr("title")),
        width: i32_(node.attr("width")),
        height: i32_(node.attr("height")),
        background: background(node),
        title_bar: flag(node.attr("titlebar")),
        resizable: flag(node.attr("resizable")),
        timer_interval: node
            .attr("timerinterval")
            .and_then(|raw| Value::parse(raw).as_i32())
            .and_then(|value| u32::try_from(value).ok()),
        script_files: node
            .attr("scriptfile")
            .map(|raw| {
                raw.split(';')
                    .map(str::trim)
                    .filter(|file| !file.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        children: node.children.iter().map(element).collect(),
    }
}

/// One element of a view, by its name. Predefined elements are their
/// plain kind with the action or binding their name promises.
fn element(node: &Node) -> Element {
    let name = node.name.as_str();
    if predefined_action(name).is_some() {
        // A predefined transport element standing on its own: the same
        // button a group would draw, with its action built in.
        return Element::Button(button(node, name));
    }
    match name {
        "subview" => Element::Subview(Subview {
            common: common(node),
            background: background(node),
            children: node.children.iter().map(element).collect(),
        }),
        "image" => Element::Image(Image {
            common: common(node),
            image: text(node.attr("image")),
            transparency_color: color(node.attr("transparencycolor")),
            tiled: flag(node.attr("tiled")),
        }),
        "button" | "imagebutton" => Element::Button(button(node, name)),
        "buttongroup" => Element::ButtonGroup(group(node)),
        "slider" | "seekslider" | "volumeslider" | "balanceslider" => {
            Element::Slider(slider(node, name))
        }
        "text" | "currentpositiontext" | "durationtext" | "statustext" => {
            Element::Text(text_element(node, name))
        }
        other => Element::Other(Other {
            name: other.to_string(),
            common: common(node),
            children: node.children.iter().map(element).collect(),
        }),
    }
}

fn button(node: &Node, name: &str) -> Button {
    Button {
        common: common(node),
        states: ButtonStates {
            image: text(node.attr("image")),
            hover: text(node.attr("hoverimage")),
            down: text(node.attr("downimage")),
            hover_down: text(node.attr("hoverdownimage")),
            disabled: text(node.attr("disabledimage")),
        },
        transparency_color: color(node.attr("transparencycolor")),
        sticky: flag(node.attr("sticky")),
        tiled: flag(node.attr("tiled")),
        action: predefined_action(name).unwrap_or_else(|| {
            node.attr("onclick")
                .map_or(Action::None, action_from_handler)
        }),
    }
}

fn group(node: &Node) -> ButtonGroup {
    ButtonGroup {
        common: common(node),
        states: ButtonStates {
            image: text(node.attr("image")),
            hover: text(node.attr("hoverimage")),
            down: text(node.attr("downimage")),
            hover_down: text(node.attr("hoverdownimage")),
            disabled: text(node.attr("disabledimage")),
        },
        mapping_image: text(node.attr("mappingimage")),
        transparency_color: color(node.attr("transparencycolor")),
        radio: flag(node.attr("radio")),
        // The group's bitmap shows behind its buttons unless the skin
        // asks for buttons alone.
        show_background: node
            .attr("showbackground")
            .is_none_or(|raw| Value::parse(raw).as_bool().unwrap_or(true)),
        buttons: node
            .children
            .iter()
            .filter(|child| {
                child.name == "buttonelement" || predefined_action(&child.name).is_some()
            })
            .map(|child| ButtonElement {
                id: text(child.attr("id")),
                mapping_color: color(child.attr("mappingcolor")),
                sticky: flag(child.attr("sticky")),
                tooltip: text(child.attr("tooltip")).or_else(|| text(child.attr("uptooltip"))),
                enabled: child.attr("enabled").map(Value::parse),
                action: predefined_action(&child.name).unwrap_or_else(|| {
                    child
                        .attr("onclick")
                        .map_or(Action::None, action_from_handler)
                }),
            })
            .collect(),
    }
}

fn slider(node: &Node, name: &str) -> Slider {
    let value = node.attr("value").map(Value::parse);
    Slider {
        common: common(node),
        background_image: text(node.attr("backgroundimage")),
        background_hover_image: text(node.attr("backgroundhoverimage")),
        foreground_image: text(node.attr("foregroundimage")),
        foreground_hover_image: text(node.attr("foregroundhoverimage")),
        thumb_image: text(node.attr("thumbimage")),
        thumb_hover_image: text(node.attr("thumbhoverimage")),
        thumb_down_image: text(node.attr("thumbdownimage")),
        thumb_disabled_image: text(node.attr("thumbdisabledimage")),
        direction: match node
            .attr("direction")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("vertical") => Direction::Vertical,
            _ => Direction::Horizontal,
        },
        tiled: node
            .attr("tiled")
            .is_none_or(|raw| Value::parse(raw).as_bool().unwrap_or(true)),
        border_size: i32_(node.attr("bordersize")).unwrap_or(0),
        min: node.attr("min").map(Value::parse),
        max: node.attr("max").map(Value::parse),
        value: value.clone(),
        use_foreground_progress: node
            .attr("useforegroundprogress")
            .is_none_or(|raw| Value::parse(raw).as_bool().unwrap_or(true)),
        foreground_progress: node.attr("foregroundprogress").map(Value::parse),
        transparency_color: color(node.attr("transparencycolor")),
        // An explicit binding wins over the one the element's name
        // promises, since the skin wrote both.
        binding: value
            .as_ref()
            .and_then(Value::binding)
            .or_else(|| predefined_binding(name)),
    }
}

fn text_element(node: &Node, name: &str) -> Text {
    let value = node.attr("value").map(Value::parse);
    let styles = node.attr("fontstyle").map(|raw| raw.to_ascii_lowercase());
    let has = |style: &str| styles.as_deref().is_some_and(|all| all.contains(style));
    let scrolling = flag(node.attr("scrolling")).then(|| Scrolling {
        amount: i32_(node.attr("scrollingamount")).unwrap_or(1),
        delay_ms: node
            .attr("scrollingdelay")
            .and_then(|raw| Value::parse(raw).as_i32())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(50),
        direction: match node
            .attr("scrollingdirection")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("right") => ScrollDirection::Right,
            _ => ScrollDirection::Left,
        },
    });
    Text {
        common: common(node),
        value: value.clone(),
        font_face: text(node.attr("fontface")),
        font_size: node.attr("fontsize").map(Value::parse),
        font_style: FontStyle {
            bold: has("bold"),
            italic: has("italic"),
            underline: has("underline"),
            strikeout: has("strikeout"),
        },
        foreground_color: color(node.attr("foregroundcolor")),
        background_color: color(node.attr("backgroundcolor")),
        justification: match node
            .attr("justification")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("center") => Justification::Center,
            Some("right") => Justification::Right,
            _ => Justification::Left,
        },
        scrolling,
        word_wrap: flag(node.attr("wordwrap")),
        binding: value
            .as_ref()
            .and_then(Value::binding)
            .or_else(|| predefined_binding(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wmp::xml;

    fn document(text: &str) -> (Theme, Vec<View>) {
        let nodes = xml::parse(text.as_bytes()).unwrap();
        theme(&nodes).unwrap()
    }

    #[test]
    fn a_toothy_shaped_skin_is_read_whole() {
        let (theme, views) = document(
            r##"<theme id="toothy" author="Microsoft" copyright="©2000">
                <view width="586" height="335" backgroundColor="none"
                    titleBar="false" resizable="false" timerInterval="333"
                    scriptFile="toothy.js;res://wmploc.dll/RT_TEXT/#132;">
                    <subview left="0" top="0" backgroundImage="toothy_base.bmp"
                        transparencyColor="#FF00FF">
                        <buttongroup left="27" top="252"
                            mappingImage="playcontrols_map.bmp"
                            image="playcontrols_01_Df.bmp"
                            hoverImage="playcontrols_02_RV.bmp"
                            downImage="playcontrols_03_Dwn.bmp">
                            <prevelement mappingColor="#00FF00"/>
                            <playelement mappingColor="#0000FF" id="bgPlay"/>
                            <stopelement mappingColor="#FFFF00"/>
                            <nextelement mappingColor="#00FFFF"/>
                            <buttonelement mappingColor="#EE0000" upToolTip="View playlist"
                                onClick="TogglePl();"/>
                        </buttongroup>
                        <pausebutton left="123" top="253" visible="false"
                            image="pause_01_DF.bmp" upToolTip="Play"/>
                        <slider id="seek" left="100" top="135" height="32" width="151"
                            direction="Horizontal" tiled="true"
                            backgroundImage="progressbar.bmp"
                            thumbImage="tooth_seek_thumb.bmp"
                            transparencyColor="#FF00FF" borderSize="20" min="0"
                            max="wmpprop:player.currentmedia.duration"
                            value="wmpprop:player.Controls.currentPosition"
                            onDragEnd="player.controls.currentposition=value;"/>
                        <text id="metadataTitle" left="141" top="290" width="138"
                            foregroundColor="#FFFF33" fontSize="8" fontStyle="bold"
                            value="wmpprop:player.currentmedia.name"/>
                        <text value="Treble" left="jscript:treble.left+treble.width/2-15;"/>
                    </subview>
                </view>
            </theme>"##,
        );
        assert_eq!(theme.author.as_deref(), Some("Microsoft"));
        assert_eq!(theme.copyright.as_deref(), Some("©2000"));
        let view = &views[0];
        assert_eq!((view.width, view.height), (Some(586), Some(335)));
        assert!(!view.title_bar);
        assert_eq!(view.timer_interval, Some(333));
        assert_eq!(
            view.script_files,
            ["toothy.js", "res://wmploc.dll/RT_TEXT/#132"]
        );
        let subview = match &view.children[0] {
            Element::Subview(subview) => subview,
            other => panic!("expected a subview, got {other:?}"),
        };
        assert_eq!(subview.background.image.as_deref(), Some("toothy_base.bmp"));
        assert_eq!(subview.background.transparency_color, Some([0xFF, 0, 0xFF]));
        assert_eq!(subview.children.len(), 5);

        let group = match &subview.children[0] {
            Element::ButtonGroup(group) => group,
            other => panic!("expected a button group, got {other:?}"),
        };
        assert_eq!(group.mapping_image.as_deref(), Some("playcontrols_map.bmp"));
        let mapped: Vec<_> = group
            .buttons
            .iter()
            .map(|button| (button.mapping_color, &button.action))
            .collect();
        assert_eq!(
            mapped,
            [
                (Some([0, 0xFF, 0]), &Action::Previous),
                (Some([0, 0, 0xFF]), &Action::Play),
                (Some([0xFF, 0xFF, 0]), &Action::Stop),
                (Some([0, 0xFF, 0xFF]), &Action::Next),
                (Some([0xEE, 0, 0]), &Action::Unhandled("TogglePl();".into())),
            ]
        );
        assert_eq!(group.buttons[1].id.as_deref(), Some("bgPlay"));
        assert_eq!(group.buttons[4].tooltip.as_deref(), Some("View playlist"));

        let pause = match &subview.children[1] {
            Element::Button(button) => button,
            other => panic!("expected a button, got {other:?}"),
        };
        assert_eq!(pause.action, Action::Pause);
        assert_eq!(pause.common.visible_bool(), Some(false));
        assert_eq!(pause.common.left_i32(), Some(123));

        let seek = match &subview.children[2] {
            Element::Slider(slider) => slider,
            other => panic!("expected a slider, got {other:?}"),
        };
        assert_eq!(seek.binding, Some(Binding::Position));
        assert_eq!(
            seek.max.as_ref().and_then(Value::binding),
            Some(Binding::Duration)
        );
        assert_eq!(seek.border_size, 20);
        assert_eq!(seek.direction, Direction::Horizontal);
        assert!(seek.tiled);

        let title = match &subview.children[3] {
            Element::Text(text) => text,
            other => panic!("expected text, got {other:?}"),
        };
        assert_eq!(title.binding, Some(Binding::TrackName));
        assert!(title.font_style.bold);
        assert_eq!(title.foreground_color, Some([0xFF, 0xFF, 0x33]));

        let label = match &subview.children[4] {
            Element::Text(text) => text,
            other => panic!("expected text, got {other:?}"),
        };
        assert_eq!(
            label.common.left,
            Some(Value::JScript("treble.left+treble.width/2-15".into()))
        );
        assert_eq!(label.binding, None);
    }

    #[test]
    fn the_elements_a_skin_does_not_draw_are_kept_with_their_children() {
        let (_, views) = document(
            "<theme><view><effects/><playlist id=\"pl\"><column/></playlist>\
             <wmpvideo/></view></theme>",
        );
        let others: Vec<&Other> = views[0]
            .children
            .iter()
            .map(|element| match element {
                Element::Other(other) => other,
                other => panic!("expected other, got {other:?}"),
            })
            .collect();
        assert_eq!(
            others
                .iter()
                .map(|other| other.name.as_str())
                .collect::<Vec<_>>(),
            ["effects", "playlist", "wmpvideo"]
        );
        assert_eq!(others[1].common.id.as_deref(), Some("pl"));
        assert_eq!(others[1].children.len(), 1);
    }

    #[test]
    fn predefined_controls_carry_their_bindings_and_a_written_one_wins() {
        let (_, views) = document(
            "<theme><view>\
             <seekslider/><volumeslider value=\"wmpprop:player.settings.volume\"/>\
             <currentpositiontext/><durationtext/><statustext/>\
             <slider value=\"wmpprop:player.settings.balance\"/>\
             </view></theme>",
        );
        let bindings: Vec<Option<Binding>> = views[0]
            .children
            .iter()
            .map(|element| match element {
                Element::Slider(slider) => slider.binding.clone(),
                Element::Text(text) => text.binding.clone(),
                other => panic!("expected a control, got {other:?}"),
            })
            .collect();
        assert_eq!(
            bindings,
            [
                Some(Binding::Position),
                Some(Binding::Volume),
                Some(Binding::PositionString),
                Some(Binding::DurationString),
                Some(Binding::Status),
                Some(Binding::Balance),
            ]
        );
    }

    #[test]
    fn handlers_are_read_as_the_actions_they_name() {
        let cases = [
            ("view.minimize();", Action::Minimize),
            ("view.close();", Action::Close),
            ("view.ReturnToMediaCenter();", Action::ReturnToMediaCenter),
            ("vMain.minimize();", Action::Minimize),
            ("player.controls.next()", Action::Next),
            ("player.controls.previous();", Action::Previous),
            ("player.controls.play();", Action::Play),
            ("player.controls.pause();", Action::Pause),
            ("player.controls.stop();", Action::Stop),
            ("player.settings.mute=down;", Action::Mute),
            ("eq.reset();", Action::ResetEq),
            ("visEffects.previous();", Action::EffectsPrevious),
            ("visEffects.next();", Action::EffectsNext),
            ("theme.openView('vPl');", Action::OpenView("vPl".into())),
            (
                "theme.closeView(\"vSettings\");",
                Action::CloseView("vSettings".into()),
            ),
            ("TogglePl();", Action::Unhandled("TogglePl();".into())),
            (
                "SetVisibility(noPane);",
                Action::Unhandled("SetVisibility(noPane);".into()),
            ),
        ];
        for (handler, expected) in cases {
            assert_eq!(action_from_handler(handler), expected, "for {handler:?}");
        }
    }

    #[test]
    fn eq_bands_bind_by_number_and_stray_ones_do_not() {
        assert_eq!(binding_from_path("eq.gainLevel10"), Binding::EqGain(10));
        assert_eq!(binding_from_path("eq.gainlevel1"), Binding::EqGain(1));
        assert_eq!(
            binding_from_path("eq.gainlevel11"),
            Binding::Unhandled("eq.gainlevel11".into())
        );
        assert_eq!(
            binding_from_path("player.something.else"),
            Binding::Unhandled("player.something.else".into())
        );
    }

    #[test]
    fn values_are_sorted_into_their_kinds() {
        assert_eq!(Value::parse(" 586 "), Value::Literal("586".into()));
        assert_eq!(
            Value::parse("wmpprop:player.settings.volume"),
            Value::WmpProp("player.settings.volume".into())
        );
        assert_eq!(
            Value::parse("WMPPROP:player.settings.volume;"),
            Value::WmpProp("player.settings.volume".into())
        );
        assert_eq!(
            Value::parse("jscript:treble.top+19;"),
            Value::JScript("treble.top+19".into())
        );
        assert_eq!(
            Value::parse("jscript:  balance.left ; "),
            Value::JScript("balance.left".into())
        );
        assert_eq!(Value::parse("Arial"), Value::Literal("Arial".into()));
    }

    #[test]
    fn numbers_and_flags_come_out_of_literals_only() {
        assert_eq!(Value::parse("12").as_i32(), Some(12));
        assert_eq!(Value::parse("wmpprop:x").as_i32(), None);
        assert_eq!(Value::parse("true").as_bool(), Some(true));
        assert_eq!(Value::parse("0").as_bool(), Some(false));
        assert_eq!(Value::parse("on").as_bool(), None);
        assert_eq!(Value::parse("1.5").as_f64(), Some(1.5));
    }

    #[test]
    fn colours_are_read_as_skins_write_them() {
        assert_eq!(color(Some("#FF00FF")), Some([0xFF, 0, 0xFF]));
        assert_eq!(color(Some("ff00ff")), Some([0xFF, 0, 0xFF]));
        assert_eq!(color(Some("#F0F")), Some([0xFF, 0, 0xFF]));
        assert_eq!(color(Some("#FFFF3300")), None);
        assert_eq!(color(Some("none")), None);
        assert_eq!(color(Some("white")), None);
        assert_eq!(color(None), None);
    }

    #[test]
    fn a_definition_without_a_theme_is_named() {
        assert!(theme(&xml::parse(b"<view/>").unwrap()).is_err());
        assert!(matches!(xml::parse(b""), Err(xml::ParseError::Empty)));
    }

    #[test]
    fn a_view_holds_its_common_attributes() {
        let (_, views) = document(
            "<theme><view id=\"vMain\" title=\"Player\" width=\"256\" height=\"100\"\
             titleBar=\"true\" resizable=\"true\"/></theme>",
        );
        let view = &views[0];
        assert_eq!(view.id.as_deref(), Some("vMain"));
        assert_eq!(view.title.as_deref(), Some("Player"));
        assert!(view.title_bar);
        assert!(view.resizable);
    }
}
