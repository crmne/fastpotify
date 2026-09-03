//! Layout arithmetic: the `jscript:` values skins write into geometry
//! attributes — `treble.left+treble.width/2-15` — and the odd
//! visibility written as an expression. The arithmetic is all the
//! scripting most skins need for where things sit; anything a script
//! would compute at runtime stays where its attributes put it.

use std::collections::{HashMap, HashSet};

use super::ir::{Common, Value, View};

/// One of the four geometry values, as attributes name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attr {
    Left,
    Top,
    Width,
    Height,
}

impl Attr {
    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Top => 1,
            Self::Width => 2,
            Self::Height => 3,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "left" | "x" => Some(Self::Left),
            "top" | "y" => Some(Self::Top),
            "width" => Some(Self::Width),
            "height" => Some(Self::Height),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Top => "top",
            Self::Width => "width",
            Self::Height => "height",
        }
    }
}

/// What the view's elements resolved to, by id: each of the four
/// geometry values, as far as arithmetic could settle it. Values that
/// were written as numbers stand as written; values written as
/// expressions resolve against the others, in whatever order they
/// depend on each other.
#[derive(Debug, Default)]
pub struct Layout {
    resolved: HashMap<String, [Option<i32>; 4]>,
    /// Expressions that could not be resolved, warned once.
    warned: HashSet<String>,
    /// The skin's own numbers, declared as script globals: a name in
    /// an expression that is not an element, nor an attribute, answers
    /// from here (`left="jscript:leftOffset;"`).
    constants: HashMap<String, i32>,
}

impl Layout {
    /// Resolves the view's geometry once, when the skin is worn. The
    /// view itself answers to its id and to the keyword `view`, the way
    /// skins write it.
    pub fn build(view: &View) -> Self {
        Self::build_with(view, HashMap::new())
    }

    /// Builds the layout with the skin's declared constants, so the
    /// expressions that name a script's `var` resolve against it.
    pub fn build_with(view: &View, constants: HashMap<String, i32>) -> Self {
        let mut resolved = HashMap::new();
        let view_entry = [Some(0), Some(0), view.width, view.height];
        if let Some(id) = &view.id {
            resolved.insert(id.clone(), view_entry);
        }
        resolved.insert("view".to_string(), view_entry);

        // Every element's numbers as written, and the expressions still
        // to settle, in the order the walk found them.
        let mut exprs: Vec<(String, usize, String)> = Vec::new();
        walk(view, &mut |common| {
            let Some(id) = &common.id else { return };
            let entry = resolved.entry(id.clone()).or_insert([None; 4]);
            for (value, index) in [
                (&common.left, 0usize),
                (&common.top, 1),
                (&common.width, 2),
                (&common.height, 3),
            ] {
                match value {
                    Some(Value::Literal(text)) => {
                        entry[index] = text.trim().parse().ok();
                    }
                    Some(Value::JScript(expr)) => exprs.push((id.clone(), index, expr.clone())),
                    Some(Value::WmpProp(_)) => {}
                    // An unset left or top stands at the origin, the way
                    // the player places it; sizes keep their fallbacks.
                    None if index < 2 => {
                        entry[index] = Some(0);
                    }
                    None => {}
                }
            }
        });

        // Expressions name other elements, including ones that are
        // themselves expressions. Settling goes round until a pass
        // settles nothing new; the cap keeps a skin that names itself
        // from circling forever.
        for _ in 0..=exprs.len() {
            let mut changed = false;
            for (id, index, expr) in &exprs {
                let entry = resolved.get(id).expect("collected ids are entries");
                if entry[*index].is_some() {
                    continue;
                }
                let look = |name: &str, attr: &str| {
                    // A bare attribute name answers from the element
                    // itself: its own numbers as they settle, round by
                    // round. A bare name that is no attribute and no
                    // element answers from the skin's declared numbers.
                    if attr.is_empty() {
                        if let Some(attr) = Attr::from_name(name) {
                            return resolved.get(id).and_then(|entry| entry[attr.index()]);
                        }
                        return constants.get(&name.to_ascii_lowercase()).copied();
                    }
                    Attr::from_name(attr)
                        .and_then(|attr| resolved.get(name).and_then(|entry| entry[attr.index()]))
                };
                if let Some(number) = eval(expr, &look) {
                    resolved.get_mut(id).expect("same entry")[*index] = Some(number);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let mut warned = HashSet::new();
        for (id, index, expr) in &exprs {
            if resolved.get(id).expect("same entry")[*index].is_none() {
                log_or_warn(&mut warned, expr, *index);
            }
        }
        Self {
            resolved,
            warned,
            constants,
        }
    }

    /// One geometry value: a number as written, or the expression that
    /// computed it. Nothing resolvable stands in as nothing, and the
    /// caller falls back as its element demands.
    pub fn number(&mut self, common: &Common, attr: Attr) -> Option<i32> {
        let value = match attr {
            Attr::Left => &common.left,
            Attr::Top => &common.top,
            Attr::Width => &common.width,
            Attr::Height => &common.height,
        };
        match value.as_ref()? {
            Value::Literal(text) => text.trim().parse().ok(),
            Value::WmpProp(_) => None,
            Value::JScript(expr) => {
                let id = common.id.as_deref();
                if let Some(number) = id
                    .and_then(|id| self.resolved.get(id))
                    .and_then(|entry| entry[attr.index()])
                {
                    return Some(number);
                }
                let look = |name: &str, attr: &str| {
                    if attr.is_empty() {
                        // A bare attribute answers from the element
                        // itself; a bare name that is neither answers
                        // from the skin's declared numbers.
                        Self::own(common, self, id, name)
                            .or_else(|| self.constants.get(&name.to_ascii_lowercase()).copied())
                    } else {
                        Attr::from_name(attr).and_then(|attr| {
                            self.resolved
                                .get(name)
                                .and_then(|entry| entry[attr.index()])
                        })
                    }
                };
                match eval(expr, &look) {
                    Some(number) => Some(number),
                    None => {
                        log_or_warn(&mut self.warned, expr, attr.index());
                        None
                    }
                }
            }
        }
    }

    /// A bare attribute name answers from the element itself: what the
    /// layout settled for it, or what it wrote outright when the layout
    /// never heard of it - an element without an id settles nothing
    /// ahead of time.
    fn own(common: &Common, layout: &Layout, id: Option<&str>, name: &str) -> Option<i32> {
        let attr = Attr::from_name(name)?;
        let own = match attr {
            Attr::Left => &common.left,
            Attr::Top => &common.top,
            Attr::Width => &common.width,
            Attr::Height => &common.height,
        };
        if let Some(Value::Literal(text)) = own.as_ref() {
            return text.trim().parse().ok();
        }
        id.and_then(|id| layout.resolved.get(id))
            .and_then(|entry| entry[attr.index()])
    }

    /// Whether a visibility expression says an element shows. A number
    /// stands for its truth: anything but zero shows.
    pub fn truth(
        &mut self,
        value: &Option<Value>,
        id: Option<&str>,
        playstate: Option<i32>,
    ) -> Option<bool> {
        let expr = match value.as_ref()? {
            Value::JScript(expr) => expr,
            _ => return None,
        };
        let look = |name: &str, attr: &str| {
            // A bare attribute name answers from the element itself;
            // anything else falls through to the states below.
            if attr.is_empty() && Attr::from_name(name).is_some() {
                return Attr::from_name(name).and_then(|attr| {
                    id.and_then(|id| self.resolved.get(id))
                        .and_then(|entry| entry[attr.index()])
                });
            }
            if name.eq_ignore_ascii_case("player") {
                return if attr.eq_ignore_ascii_case("playstate") {
                    playstate
                } else {
                    None
                };
            }
            if name.eq_ignore_ascii_case("psundefined") {
                return Some(0);
            }
            if name.eq_ignore_ascii_case("psstopped") {
                return Some(1);
            }
            if name.eq_ignore_ascii_case("pspaused") {
                return Some(2);
            }
            if name.eq_ignore_ascii_case("psplaying") {
                return Some(3);
            }
            if name.eq_ignore_ascii_case(id.unwrap_or("")) {
                return None;
            }
            Attr::from_name(attr).and_then(|attr| {
                self.resolved
                    .get(name)
                    .and_then(|entry| entry[attr.index()])
            })
        };
        let number = eval(expr, &look)?;
        Some(number != 0)
    }
}

/// The player's state as WMP numbers it: playing, paused (a remembered
/// song waits), or stopped, with `psUndefined` at zero.
pub fn playstate(playing: bool, resuming: bool) -> i32 {
    if playing {
        3
    } else if resuming {
        2
    } else {
        1
    }
}

fn log_or_warn(warned: &mut HashSet<String>, expr: &str, index: usize) {
    if warned.insert(expr.to_string()) {
        log::warn!(
            "WMP skin: {}=\"jscript:{}\" does not resolve; 0 stands in",
            Attr::from_index(index).map_or("?", Attr::name),
            expr
        );
    }
}

impl Attr {
    fn from_index(index: usize) -> Option<Self> {
        [Self::Left, Self::Top, Self::Width, Self::Height]
            .get(index)
            .copied()
    }
}

/// Every element in the view, in the order it is drawn, through the
/// subviews it nests in.
fn walk(view: &View, visit: &mut impl FnMut(&Common)) {
    fn children(elements: &[super::ir::Element], visit: &mut impl FnMut(&Common)) {
        for element in elements {
            visit(element.common());
            if let super::ir::Element::Subview(subview) = element {
                children(&subview.children, visit);
            }
        }
    }
    children(&view.children, visit);
}

/// The words an expression is made of.
#[derive(Clone)]
enum Token {
    Number(i64),
    Name(String),
    Equal,
    NotEqual,
    Plus,
    Minus,
    Star,
    Slash,
    Open,
    Close,
}

fn tokens(expr: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = expr;
    'scan: while !rest.is_empty() {
        rest = rest.trim_start();
        let Some(first) = rest.chars().next() else {
            break;
        };
        let token = match first {
            // The dot between a name and its attribute is not a token;
            // the parser sees the two words back to back.
            '.' => {
                rest = &rest[1..];
                continue;
            }
            '+' => Some((Token::Plus, 1)),
            '-' => Some((Token::Minus, 1)),
            '*' => Some((Token::Star, 1)),
            '/' => Some((Token::Slash, 1)),
            '(' => Some((Token::Open, 1)),
            ')' => Some((Token::Close, 1)),
            '=' if rest.starts_with("==") => Some((Token::Equal, 2)),
            '!' if rest.starts_with("!=") => Some((Token::NotEqual, 2)),
            digit if digit.is_ascii_digit() => {
                let end = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                match rest[..end].parse::<i64>() {
                    Ok(number) => Some((Token::Number(number), end)),
                    Err(_) => None,
                }
            }
            letter if letter.is_ascii_alphanumeric() || letter == '_' => {
                let end = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                Some((Token::Name(rest[..end].to_string()), end))
            }
            _ => None,
        };
        match token {
            Some((token, width)) => {
                tokens.push(token);
                rest = &rest[width..];
            }
            None => break 'scan,
        }
    }
    tokens
}

/// An expression as a number, against what names resolve to. Any word
/// it cannot settle, or arithmetic it cannot carry — a division by
/// zero, a name that is nothing — leaves the whole as nothing.
fn eval(expr: &str, look: &dyn Fn(&str, &str) -> Option<i32>) -> Option<i32> {
    let tokens = tokens(expr);
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        look,
    };
    let number = parser.comparison()?;
    if parser.at != tokens.len() {
        return None;
    }
    i32::try_from(number).ok()
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    look: &'a dyn Fn(&str, &str) -> Option<i32>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn take(&mut self) -> Option<&'a Token> {
        let token = self.tokens.get(self.at)?;
        self.at += 1;
        Some(token)
    }

    /// Equality, the loosest the arithmetic carries, so a visibility
    /// like `player.playstate != psUndefined` reads whole.
    fn comparison(&mut self) -> Option<i64> {
        let left = self.sum()?;
        match self.peek() {
            Some(Token::Equal) => {
                self.at += 1;
                Some((left == self.sum()?) as i64)
            }
            Some(Token::NotEqual) => {
                self.at += 1;
                Some((left != self.sum()?) as i64)
            }
            _ => Some(left),
        }
    }

    fn sum(&mut self) -> Option<i64> {
        let mut left = self.product()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.at += 1;
                    left = left.checked_add(self.product()?)?;
                }
                Some(Token::Minus) => {
                    self.at += 1;
                    left = left.checked_sub(self.product()?)?;
                }
                _ => return Some(left),
            }
        }
    }

    fn product(&mut self) -> Option<i64> {
        let mut left = self.unary()?;
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.at += 1;
                    left = left.checked_mul(self.unary()?)?;
                }
                Some(Token::Slash) => {
                    self.at += 1;
                    let right = self.unary()?;
                    if right == 0 {
                        return None;
                    }
                    left = left.checked_div(right)?;
                }
                _ => return Some(left),
            }
        }
    }

    fn unary(&mut self) -> Option<i64> {
        if matches!(self.peek(), Some(Token::Minus)) {
            self.at += 1;
            Some(self.unary()?.checked_neg()?)
        } else {
            self.primary()
        }
    }

    fn primary(&mut self) -> Option<i64> {
        match self.take()? {
            Token::Number(number) => Some(*number),
            Token::Open => {
                let inner = self.comparison()?;
                if !matches!(self.take()?, Token::Close) {
                    return None;
                }
                Some(inner)
            }
            Token::Name(name) => {
                // A reference reads `name.attr`; the dot is not part of
                // either word, so it was skipped by the scanner. A word
                // with no attribute after it — a state's name — goes to
                // the look on its own.
                let attr = match self.peek().cloned() {
                    Some(Token::Name(second)) => {
                        self.at += 1;
                        second
                    }
                    _ => String::new(),
                };
                (self.look)(name, &attr).map(i64::from)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_with(expr: &str, table: &[(&str, Attr, i32)]) -> Option<i32> {
        let look = |name: &str, attr: &str| {
            let attr = Attr::from_name(attr)?;
            table
                .iter()
                .find(|(known, known_attr, _)| *known == name && *known_attr == attr)
                .map(|(_, _, value)| *value)
        };
        eval(expr, &look)
    }

    #[test]
    fn arithmetic_reads_like_arithmetic() {
        assert_eq!(eval_with("3+4*2", &[]), Some(11));
        assert_eq!(eval_with("10/2-1", &[]), Some(4));
        assert_eq!(eval_with("(2+3)*2", &[]), Some(10));
        assert_eq!(eval_with("-5+2", &[]), Some(-3));
        assert_eq!(eval_with("7/0", &[]), None);
        assert_eq!(eval_with("1==1", &[]), Some(1));
        assert_eq!(eval_with("1!=2", &[]), Some(1));
        // A word that is nothing, or a stray word after the expression,
        // leaves the whole as nothing.
        assert_eq!(eval_with("3+", &[]), None);
        assert_eq!(eval_with("3 4", &[]), None);
        assert_eq!(eval_with("Volume", &[]), None);
    }

    #[test]
    fn references_name_other_elements_geometry() {
        let table = [("treble", Attr::Left, 40i32), ("treble", Attr::Width, 30)];
        assert_eq!(eval_with("treble.left+treble.width/2-15", &table), Some(40));
        assert_eq!(
            eval_with("eq1.left+15", &[("eq1", Attr::Left, 10)]),
            Some(25)
        );
        assert_eq!(eval_with("ghost.left", &[]), None);
    }

    fn view_with(children: &str) -> View {
        let definition = format!(
            "<theme><view id=\"main\" width=\"400\" height=\"300\">{children}</view></theme>"
        );
        let document =
            crate::wmp::SkinDocument::from_files("test", [("skin.wms", definition.into_bytes())])
                .unwrap();
        document.views.into_iter().next().unwrap()
    }

    #[test]
    fn expressions_settle_in_dependency_order() {
        let view = view_with(
            r#"<image id="a" left="10" top="0" width="30" height="20"/>
               <button id="b" left="jscript:a.left+a.width+7;"/>
               <slider id="c" left="jscript:b.left+15;"/>"#,
        );
        let layout = Layout::build(&view);
        let a = layout.resolved.get("a").unwrap();
        assert_eq!(a[0], Some(10));
        let b = layout.resolved.get("b").unwrap();
        assert_eq!(b[0], Some(47));
        let c = layout.resolved.get("c").unwrap();
        assert_eq!(c[0], Some(62));
    }

    #[test]
    fn the_view_answers_to_its_id_and_to_the_keyword() {
        let view = view_with(r#"<image id="pane" left="jscript:view.width-40;"/>"#);
        let layout = Layout::build(&view);
        assert_eq!(layout.resolved.get("pane").unwrap()[0], Some(360));
        let view = view_with(r#"<image id="pane" left="jscript:main.width/2;"/>"#);
        let layout = Layout::build(&view);
        assert_eq!(layout.resolved.get("pane").unwrap()[0], Some(200));
    }

    #[test]
    fn a_bare_attribute_answers_from_the_element_itself() {
        let view = view_with(
            r#"<subview id="cpane" left="0" top="208" width="285" height="jscript:view.height - top"/>"#,
        );
        let layout = Layout::build(&view);
        assert_eq!(layout.resolved.get("cpane").unwrap()[3], Some(92));
    }

    #[test]
    fn an_element_without_an_id_reads_its_own_literals() {
        let view = view_with(
            r#"<subview id="cpane" left="0" top="208" width="285" height="151"><subview top="55" width="285" height="jscript:cpane.height-top"/></subview>"#,
        );
        let layout = Layout::build(&view);
        assert_eq!(layout.resolved.get("cpane").unwrap()[3], Some(151));
        // The inner subview names no id, so the layout never heard of
        // it; its numbers still settle from its own literals and the
        // elements it names.
        let outer = match &view.children[0] {
            crate::wmp::ir::Element::Subview(outer) => outer,
            other => panic!("expected a subview, got {other:?}"),
        };
        assert_eq!(outer.common.id.as_deref(), Some("cpane"));
        let inner = match &outer.children[0] {
            crate::wmp::ir::Element::Subview(inner) => inner.common.clone(),
            other => panic!("expected a subview, got {other:?}"),
        };
        assert_eq!(inner.id, None);
        let mut layout = layout;
        assert_eq!(layout.number(&inner, Attr::Height), Some(96));
    }

    #[test]
    fn an_unset_left_or_top_stands_at_the_origin() {
        let view = view_with(r#"<image id="glyph" top="5"/>"#);
        let layout = Layout::build(&view);
        assert_eq!(layout.resolved.get("glyph").unwrap()[0], Some(0));
        assert_eq!(layout.resolved.get("glyph").unwrap()[2], None);
    }

    #[test]
    fn an_expression_that_names_nothing_stands_at_zero() {
        let view = view_with(r#"<image id="pane" left="jscript:ghost.left+1;"/>"#);
        let layout = Layout::build(&view);
        assert_eq!(layout.resolved.get("pane").unwrap()[0], None);
        let common = walk_first_common(&view);
        let mut layout = layout;
        assert_eq!(layout.number(&common, Attr::Left), None);
    }

    fn walk_first_common(view: &View) -> Common {
        let mut found = None;
        walk(view, &mut |common| found = Some(common.clone()));
        found.unwrap()
    }

    #[test]
    fn numbers_written_stand_as_written() {
        let view = view_with(r#"<image id="pane" left="-12" width="20"/>"#);
        let layout = Layout::build(&view);
        let common = walk_first_common(&view);
        let mut layout = layout;
        assert_eq!(layout.number(&common, Attr::Left), Some(-12));
        assert_eq!(layout.number(&common, Attr::Width), Some(20));
        assert_eq!(layout.number(&common, Attr::Height), None);
    }

    #[test]
    fn a_bare_name_that_is_a_script_number_answers_the_layout() {
        // A skin writes `left="jscript:leftOffset;"`, where leftOffset
        // is a number its script declares. The constants the skin's
        // globals settled resolve the attribute; one that names no
        // number stays nothing.
        let view = view_with(
            r#"<subview id="head" left="jscript:leftOffset;" top="jscript:vidClosedPos;"/>"#,
        );
        let constants = [("leftoffset", 260), ("vidclosedpos", 40)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let mut layout = Layout::build_with(&view, constants);
        let common = walk_first_common(&view);
        assert_eq!(layout.number(&common, Attr::Left), Some(260));
        assert_eq!(layout.number(&common, Attr::Top), Some(40));
    }

    #[test]
    fn an_expression_builds_on_a_script_number() {
        // A pane's width settles from a constant, and the arithmetic
        // the skin writes on it settles too.
        let view = view_with(
            r#"<subview id="splEqPl" left="jscript:eqPlClosedPos;" top="jscript:eqPlClosedPos" height="jscript:vidClosedPos*2;"/>"#,
        );
        let constants = [("eqplclosedpos", 300), ("vidclosedpos", 40)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let mut layout = Layout::build_with(&view, constants);
        let common = walk_first_common(&view);
        assert_eq!(layout.number(&common, Attr::Left), Some(300));
        assert_eq!(layout.number(&common, Attr::Top), Some(300));
        assert_eq!(layout.number(&common, Attr::Height), Some(80));
    }

    #[test]
    fn a_visibility_expression_reads_the_player_state() {
        let view =
            view_with(r#"<button id="b" visible="jscript:player.playstate!=psUndefined;"/>"#);
        let mut layout = Layout::build(&view);
        let common = walk_first_common(&view);
        assert_eq!(layout.truth(&common.visible, None, Some(3)), Some(true));
        assert_eq!(layout.truth(&common.visible, None, Some(0)), Some(false));
        // The player is not an element; its name settles only the state
        // the caller hands over.
        assert_eq!(layout.truth(&common.visible, None, None), None);
    }

    #[test]
    fn the_player_state_numbers_like_the_player_does() {
        assert_eq!(playstate(true, false), 3);
        assert_eq!(playstate(false, true), 2);
        assert_eq!(playstate(false, false), 1);
    }
}
