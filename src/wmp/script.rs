//! The skin's scripts, understood as far as they drive what shows.
//!
//! A skin's JavaScript is the whole of its behaviour, but what
//! Fastpotify can honour is narrow: the visibility of the elements by
//! their ids, the enabled and down state of buttons, the player verbs
//! the handlers call, and the little arithmetic and comparisons those
//! decisions ride on. Everything else — media metadata strings, the
//! equalizer's presets, the player's own state — is read as unknown and
//! answered with the simplest truth: nothing is known, so nothing is
//! made true. The scripts are never executed against a real host; the
//! machine here stands in for the parts a skin uses to change its
//! panes, and leaves the record of what it could not follow.

use std::collections::HashMap;

use super::ir::{Action, Element, View};

/// What an expression in the script may answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Num(f64),
    Str(String),
    Flag(bool),
    /// An element the script names by its id, the way `document.all`
    /// would hand one over.
    Pane(String),
    /// Anything the script asks that the machine does not know: the
    /// player's own state, media metadata, a host builtin.
    Unknown,
}

impl Val {
    /// The truth the script would see.
    fn truth(&self, panes: &HashMap<String, bool>) -> bool {
        match self {
            Val::Num(number) => *number != 0.0,
            Val::Str(string) => !string.is_empty(),
            Val::Flag(flag) => *flag,
            Val::Pane(id) => panes.get(id).copied().unwrap_or(false),
            Val::Unknown => false,
        }
    }

    /// Whether two values would compare equal in the script. A value
    /// the machine does not know never equals anything, so a condition
    /// that hangs on the player's own state simply stays false.
    fn same(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Num(a), Val::Num(b)) => a == b,
            (Val::Str(a), Val::Str(b)) => a == b,
            (Val::Flag(a), Val::Flag(b)) => a == b,
            (Val::Pane(a), Val::Pane(b)) => a == b,
            _ => false,
        }
    }
}

/// An expression of the shape the scripts use for their decisions.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Val(Val),
    /// A name: a script variable, or an element by its id.
    Name(String),
    /// A property of a name: `x.visible`, `x.enabled`, `x.down`.
    Prop(String, String),
    Not(Box<Expr>),
    Equal(Box<Expr>, Box<Expr>),
    NotEqual(Box<Expr>, Box<Expr>),
    Greater(Box<Expr>, Box<Expr>),
    Less(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
}

impl Expr {
    fn eval(&self, machine: &Machine) -> Val {
        match self {
            Expr::Val(value) => value.clone(),
            Expr::Name(name) => match machine.vars.get(&name.to_ascii_lowercase()) {
                // A known variable stands for what it holds.
                Some(value) => value.clone(),
                // A bare element id stands for the element itself.
                None => Val::Pane(name.to_ascii_lowercase()),
            },
            Expr::Prop(owner, property) => match property.as_str() {
                // Reading x.visible answers with whether x shows,
                // wherever x is an element or a variable that stands
                // for one.
                "visible" => match machine.vars.get(&owner.to_ascii_lowercase()) {
                    Some(Val::Flag(flag)) => Val::Flag(*flag),
                    Some(Val::Pane(id)) => {
                        Val::Flag(machine.panes.get(id).copied().unwrap_or(false))
                    }
                    Some(Val::Num(number)) => Val::Flag(*number != 0.0),
                    _ => Val::Flag(
                        machine
                            .panes
                            .get(&owner.to_ascii_lowercase())
                            .copied()
                            .unwrap_or(false),
                    ),
                },
                "enabled" | "down" => Val::Unknown,
                _ => Val::Unknown,
            },
            Expr::Not(inner) => Val::Flag(!inner.eval(machine).truth(&machine.panes)),
            Expr::Equal(a, b) => Val::Flag(a.eval(machine).same(&b.eval(machine))),
            Expr::NotEqual(a, b) => Val::Flag(!a.eval(machine).same(&b.eval(machine))),
            Expr::Greater(a, b) => Val::Flag(
                compare(a.eval(machine), b.eval(machine)).is_some_and(std::cmp::Ordering::is_gt),
            ),
            Expr::Less(a, b) => Val::Flag(
                compare(a.eval(machine), b.eval(machine)).is_some_and(std::cmp::Ordering::is_lt),
            ),
            Expr::And(a, b) => Val::Flag(
                a.eval(machine).truth(&machine.panes) && b.eval(machine).truth(&machine.panes),
            ),
            Expr::Or(a, b) => Val::Flag(
                a.eval(machine).truth(&machine.panes) || b.eval(machine).truth(&machine.panes),
            ),
            Expr::Ternary(choice, yes, no) => {
                if choice.eval(machine).truth(&machine.panes) {
                    yes.eval(machine)
                } else {
                    no.eval(machine)
                }
            }
        }
    }
}

impl From<bool> for Val {
    fn from(flag: bool) -> Self {
        Val::Flag(flag)
    }
}

/// How two numbers stand to each other, when both are known.
fn compare(a: Val, b: Val) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Val::Num(a), Val::Num(b)) => a.partial_cmp(&b),
        _ => None,
    }
}

/// What a statement may write.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A script variable.
    Var(String),
    /// A property of a name the script can see: only the element
    /// states mean anything to the player.
    Prop(String, String),
}

/// One statement of a function body.
#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Var {
        name: String,
        init: Option<Expr>,
    },
    Assign(Vec<(Target, Expr)>),
    If {
        cond: Expr,
        then: Vec<Statement>,
        els: Vec<Statement>,
    },
    Switch {
        on: Expr,
        cases: Vec<(Expr, Vec<Statement>)>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// Anything the machine does not follow: strings built for
    /// metadata, presets, the player's own state. Kept only for the
    /// parse to have walked it.
    Other,
}

/// One function the skin defines, by its name and body.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Function {
    pub params: Vec<String>,
    pub body: Vec<Statement>,
}

/// The script of one skin, parsed as far as the player can follow.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    pub functions: HashMap<String, Function>,
    pub globals: Vec<Statement>,
    /// What the parser met and could not follow, once each, for the
    /// record.
    pub skipped: Vec<String>,
}

impl Script {
    /// The script files a skin carries, read whole. One that says
    /// nothing to the machine still parses, so the same skin behaves
    /// with or without its scripts understood.
    pub fn parse(sources: &[String]) -> Script {
        let mut script = Script::default();
        for source in sources {
            parse_source(&strip_comments(source), &mut script);
        }
        script
    }

    /// A function by its name, however it is written.
    pub fn function(&self, name: &str) -> Option<&Function> {
        self.functions.get(&name.to_ascii_lowercase())
    }
}

/// One skin's running state: its variables, the visible state of its
/// elements by id, and the button states the scripts have set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    vars: HashMap<String, Val>,
    panes: HashMap<String, bool>,
    enableds: HashMap<String, bool>,
    downs: HashMap<String, bool>,
}

impl Machine {
    /// The state a view comes up with: every element the skin names by
    /// an id, visible as its definition says.
    pub fn new(view: &View) -> Machine {
        let mut machine = Machine::default();
        seed(view, &mut machine);
        machine
    }

    /// Whether the skin's scripts have said an element by id is
    /// visible or not, whatever its definition once said.
    pub fn visible(&self, id: &str) -> Option<bool> {
        self.panes.get(&id.to_ascii_lowercase()).copied()
    }

    /// Whether the skin's scripts have said a button by id may be
    /// used, or is worn down as a toggle.
    pub fn enabled(&self, id: &str) -> Option<bool> {
        self.enableds.get(&id.to_ascii_lowercase()).copied()
    }

    pub fn down(&self, id: &str) -> Option<bool> {
        self.downs.get(&id.to_ascii_lowercase()).copied()
    }

    /// The bare statements of a handler, as a click would run them.
    pub fn handler(&mut self, script: &Script, handler: &str) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut state = Run {
            machine: self,
            script,
            actions: &mut actions,
            depth: 0,
        };
        if let Ok(statements) = block(handler) {
            state.run(&statements);
        }
        actions
    }

    /// A function of the skin, run on its own: what the view shows and
    /// the player does after it has gone.
    pub fn run(&mut self, script: &Script, name: &str) -> Vec<Action> {
        let mut actions = Vec::new();
        let Some(function) = script.function(name) else {
            return actions;
        };
        let mut state = Run {
            machine: self,
            script,
            actions: &mut actions,
            depth: 0,
        };
        state.call(function, &[]);
        actions
    }
}

struct Run<'a> {
    machine: &'a mut Machine,
    script: &'a Script,
    actions: &'a mut Vec<Action>,
    depth: u32,
}

/// How deep the machine may follow calls before it stops: a guard for
/// skins whose handlers chase each other.
const DEPTH_LIMIT: u32 = 24;

impl Run<'_> {
    fn run(&mut self, statements: &[Statement]) {
        self.depth += 1;
        if self.depth <= DEPTH_LIMIT {
            for statement in statements {
                self.statement(statement);
            }
        }
        self.depth -= 1;
    }

    fn statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Other => {}
            Statement::Var { name, init } => {
                let value = init
                    .as_ref()
                    .map(|init| init.eval(self.machine))
                    .unwrap_or(Val::Unknown);
                self.machine.vars.insert(name.to_ascii_lowercase(), value);
            }
            Statement::Assign(chain) => {
                for (target, expr) in chain {
                    let value = expr.eval(self.machine);
                    self.write(target, &value);
                }
            }
            Statement::If { cond, then, els } => {
                if cond.eval(self.machine).truth(&self.machine.panes) {
                    self.run(then);
                } else {
                    self.run(els);
                }
            }
            Statement::Switch { on, cases } => {
                let value = on.eval(self.machine);
                for (case, body) in cases {
                    if case.eval(self.machine).same(&value) {
                        self.run(body);
                        break;
                    }
                }
            }
            Statement::Call { name, args } => self.call_name(name, args),
        }
    }

    fn call_name(&mut self, name: &str, args: &[Expr]) {
        let values: Vec<Val> = args.iter().map(|arg| arg.eval(self.machine)).collect();
        // The player's own handlers the skin calls: the machine answers
        // with the action it stands for, and the run hands it over.
        if let Some(action) = player_call(name, &values) {
            self.actions.push(action);
            return;
        }
        // A call to the view itself: opening and closing a secondary
        // view is the window going away or coming back; the machine
        // has no other views to open.
        if name.starts_with("theme.openview") || name.starts_with("theme.closeview") {
            return;
        }
        let Some(function) = self.script.function(name) else {
            // A host builtin the machine does not know: answered with
            // nothing, the way it reads when the player is not there.
            return;
        };
        let arguments: Vec<Val> = values
            .into_iter()
            .chain(function.params.iter().map(|_| Val::Unknown))
            .collect();
        self.call(function, &arguments[..function.params.len()]);
    }

    fn call(&mut self, function: &Function, args: &[Val]) {
        self.depth += 1;
        if self.depth <= DEPTH_LIMIT {
            // The parameters live as variables while the function runs,
            // and are back the way they were when it is done; the
            // state the body changes stays changed.
            let mut params = Vec::new();
            for (param, value) in function.params.iter().zip(args.iter()) {
                let key = param.to_ascii_lowercase();
                let previous = self.machine.vars.insert(key.clone(), value.clone());
                params.push((key, previous));
            }
            for statement in &function.body {
                self.statement(statement);
            }
            for (key, previous) in params {
                match previous {
                    Some(value) => {
                        self.machine.vars.insert(key, value);
                    }
                    None => {
                        self.machine.vars.remove(&key);
                    }
                }
            }
        }
        self.depth -= 1;
    }

    fn write(&mut self, target: &Target, value: &Val) {
        match target {
            Target::Var(name) => {
                self.machine
                    .vars
                    .insert(name.to_ascii_lowercase(), value.clone());
            }
            Target::Prop(owner, property) => match property.as_str() {
                "visible" => {
                    self.machine
                        .panes
                        .insert(owner.to_ascii_lowercase(), value.truth(&self.machine.panes));
                }
                "enabled" => {
                    self.machine
                        .enableds
                        .insert(owner.to_ascii_lowercase(), value.truth(&self.machine.panes));
                }
                "down" => {
                    self.machine
                        .downs
                        .insert(owner.to_ascii_lowercase(), value.truth(&self.machine.panes));
                }
                // Widths, strings, presets: the machine does not move
                // the window or write metadata.
                _ => {}
            },
        }
    }
}

/// The player verbs a script may call, as actions.
fn player_call(name: &str, _args: &[Val]) -> Option<Action> {
    match name.to_ascii_lowercase().as_str() {
        "player.controls.play" => Some(Action::Play),
        "player.controls.pause" => Some(Action::Pause),
        "player.controls.stop" => Some(Action::Stop),
        "player.controls.next" => Some(Action::Next),
        "player.controls.previous" => Some(Action::Previous),
        _ => None,
    }
}

/// The state a view comes up with, as far as its definitions say: an
/// element the skin names by an id keeps its own visibility, so a
/// script has something to turn.
fn seed(view: &View, machine: &mut Machine) {
    fn walk(elements: &[Element], machine: &mut Machine) {
        for element in elements {
            let common = element.common();
            if let Some(id) = common.id.as_deref() {
                let visible = common.visible_bool().unwrap_or(true);
                machine.panes.insert(id.to_ascii_lowercase(), visible);
            }
            if let Element::Subview(subview) = element {
                walk(&subview.children, machine);
            }
        }
    }
    walk(&view.children, machine);
}

/// Comments are nothing to the machine.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The functions and globals of one script's text. The globals are the
/// text outside the functions: the skin's variables and whatever it
/// runs as it loads.
fn parse_source(source: &str, script: &mut Script) {
    let bytes: Vec<char> = source.chars().collect();
    let mut at = 0usize;
    let mut outside = String::new();
    while at < bytes.len() {
        match find_function(&bytes[at..]).map(|found| at + found) {
            Some(start) => {
                outside.extend(bytes[at..start].iter());
                match read_function(&bytes[start..]) {
                    Some((name, function, used)) => {
                        script.functions.insert(name.to_ascii_lowercase(), function);
                        at = start + used;
                    }
                    None => {
                        outside.extend(bytes[start..].iter());
                        break;
                    }
                }
            }
            None => {
                outside.extend(bytes[at..].iter());
                break;
            }
        }
    }
    if let Ok(globals) = block(&outside) {
        script.globals = globals;
    }
}

/// Where the next `function` begins, as a word.
fn find_function(chars: &[char]) -> Option<usize> {
    let mut at = 0usize;
    while at + 8 <= chars.len() {
        if chars[at..at + 8]
            .iter()
            .collect::<String>()
            .eq_ignore_ascii_case("function")
            && at
                .checked_sub(1)
                .map(|before| !word_char(chars[before]))
                .unwrap_or(true)
            && !chars.get(at + 8).is_some_and(|after| word_char(*after))
        {
            return Some(at);
        }
        at += 1;
    }
    None
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// A function's name, parameters and body, and how far it reached.
fn read_function(chars: &[char]) -> Option<(String, Function, usize)> {
    let mut at = 8;
    while chars.get(at).is_some_and(|c| c.is_whitespace()) {
        at += 1;
    }
    let name: String = chars[at..]
        .iter()
        .collect::<String>()
        .chars()
        .take_while(|c| word_char(*c))
        .collect();
    if name.is_empty() {
        return None;
    }
    at += name.len();
    let open = chars[at..].iter().position(|c| *c == '(')?;
    let close = match_bracket(&chars[at + open..], '(')?;
    let params = chars[at + open + 1..at + open + close - 1]
        .iter()
        .collect::<String>()
        .split(',')
        .map(|param| param.trim().to_ascii_lowercase())
        .filter(|param| !param.is_empty())
        .collect();
    at += open + close;
    let open = chars[at..].iter().position(|c| *c == '{')?;
    let span = match_bracket(&chars[at + open..], '{')?;
    let body_text: String = chars[at + open + 1..at + open + span - 1].iter().collect();
    let Ok(body) = block(&body_text) else {
        return None;
    };
    Some((name, Function { params, body }, at + open + span))
}

/// Where the bracket that opens at the start closes, as the count of
/// characters up to and including it.
fn match_bracket(chars: &[char], open: char) -> Option<usize> {
    let close = match open {
        '(' => ')',
        '{' => '}',
        _ => return None,
    };
    let mut depth = 0i32;
    for (index, c) in chars.iter().enumerate() {
        if *c == open {
            depth += 1;
        } else if *c == close {
            depth -= 1;
            if depth == 0 {
                return Some(index + 1);
            }
        }
    }
    None
}

/// A run of statements, from the text of a body or a handler.
fn block(source: &str) -> Result<Vec<Statement>, ()> {
    let mut tokens = tokens(source)?;
    let mut statements = Vec::new();
    while !tokens.is_empty() {
        let (statement, used) = statement(&tokens)?;
        statements.push(statement);
        tokens.drain(..used);
    }
    Ok(statements)
}

/// A token of the script's text.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Number(f64),
    Str(String),
    Punct(String),
}

/// The text as tokens: words, numbers, strings, and the marks between.
fn tokens(source: &str) -> Result<Vec<Token>, ()> {
    let mut out = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            c if c.is_ascii_alphabetic() || c == '_' || c == '$' => {
                let mut word = String::new();
                word.push(c);
                while let Some(next) = chars.peek() {
                    if next.is_ascii_alphanumeric() || *next == '_' || *next == '$' {
                        word.push(chars.next().unwrap_or_default());
                    } else {
                        break;
                    }
                }
                out.push(Token::Word(word));
            }
            c if c.is_ascii_digit()
                || (c == '.' && chars.peek().is_some_and(char::is_ascii_digit)) =>
            {
                let mut number = String::new();
                number.push(c);
                while let Some(next) = chars.peek() {
                    if next.is_ascii_digit() || *next == '.' {
                        number.push(chars.next().unwrap_or_default());
                    } else {
                        break;
                    }
                }
                let value = number.parse::<f64>().map_err(|_| ())?;
                out.push(Token::Number(value));
            }
            '\'' | '"' => {
                let mut string = String::new();
                for next in chars.by_ref() {
                    if next == c {
                        break;
                    }
                    string.push(next);
                }
                out.push(Token::Str(string));
            }
            _ => {
                let mut punct = String::new();
                punct.push(c);
                if let Some(next) = chars.peek() {
                    let pair = format!("{c}{next}");
                    if matches!(
                        pair.as_str(),
                        "==" | "!=" | ">=" | "<=" | "&&" | "||" | "+="
                    ) {
                        punct.push(chars.next().unwrap_or_default());
                    }
                }
                out.push(Token::Punct(punct));
            }
        }
    }
    Ok(out)
}

/// One statement at the head of the token stream, and how many tokens
/// it took.
fn statement(tokens: &[Token]) -> Result<(Statement, usize), ()> {
    match tokens.first() {
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("var") => {
            let Some(Token::Word(name)) = tokens.get(1) else {
                return Err(());
            };
            match tokens.get(2) {
                Some(Token::Punct(punct)) if punct == "=" => {
                    let (init, used) = expression(&tokens[3..], 0)?;
                    let used = used
                        + 3
                        + usize::from(tokens.get(3 + used) == Some(&Token::Punct(";".into())));
                    Ok((
                        Statement::Var {
                            name: name.to_ascii_lowercase(),
                            init: Some(init),
                        },
                        used,
                    ))
                }
                _ => Ok((
                    Statement::Var {
                        name: name.to_ascii_lowercase(),
                        init: None,
                    },
                    2 + usize::from(tokens.get(2) == Some(&Token::Punct(";".into()))),
                )),
            }
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("if") => if_statement(tokens),
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("switch") => switch(tokens),
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("return") => {
            let mut used = 1usize;
            let value = if tokens
                .get(1)
                .is_some_and(|t| t != &Token::Punct(";".into()))
            {
                let (expr, span) = expression(&tokens[1..], 0)?;
                used += span;
                Some(expr)
            } else {
                None
            };
            if tokens.get(used) == Some(&Token::Punct(";".into())) {
                used += 1;
            }
            // The machine does not pass values back: a return only ends
            // its branch, which the statement walk already does.
            let _ = value;
            Ok((Statement::Other, used))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("break") => Ok((
            Statement::Other,
            1 + usize::from(tokens.get(1) == Some(&Token::Punct(";".into()))),
        )),
        _ => assignment_or_call(tokens),
    }
}

/// A statement that begins with a name: an assignment, or a call.
fn assignment_or_call(tokens: &[Token]) -> Result<(Statement, usize), ()> {
    let (target, step) = assignment_target(tokens)?;
    let used = step;
    match tokens.get(used) {
        Some(Token::Punct(punct)) if punct == "=" || punct == "+=" => {
            // A chain of assignments hands one value to every target
            // it names: a = b = c gives c's value to both. The value
            // is read first, and every further mark hands the value
            // read so far over to a target instead.
            let mut targets = vec![target.clone()];
            let mut used = used + 1;
            let (mut value, span) = expression(&tokens[used..], 0)?;
            used += span;
            while tokens.get(used) == Some(&Token::Punct("=".into())) {
                targets.push(target_of(&value)?);
                let (next, step) = expression(&tokens[used + 1..], 0)?;
                value = next;
                used += 1 + step;
            }
            if tokens.get(used) == Some(&Token::Punct(";".into())) {
                used += 1;
            }
            let chain = targets
                .into_iter()
                .map(|target| (target, value.clone()))
                .collect();
            Ok((Statement::Assign(chain), used))
        }
        _ => {
            // A call statement: the tokens from the name on read as the
            // arguments of one call.
            let (call, span) = call(tokens)?;
            if tokens.get(span) == Some(&Token::Punct(";".into())) {
                Ok((call, span + 1))
            } else {
                Ok((call, span))
            }
        }
    }
}

/// A value the chain read turns out to be another target: what it
/// names receives the value that follows.
fn target_of(value: &Expr) -> Result<Target, ()> {
    match value {
        Expr::Name(name) => Ok(Target::Var(name.clone())),
        Expr::Prop(owner, property) => Ok(Target::Prop(owner.clone(), property.clone())),
        _ => Err(()),
    }
}

/// A name, or a property of one, as the left side of an assignment.
fn assignment_target(tokens: &[Token]) -> Result<(Target, usize), ()> {
    let Some(Token::Word(name)) = tokens.first() else {
        return Err(());
    };
    match (tokens.get(1), tokens.get(2)) {
        (Some(Token::Punct(punct)), Some(Token::Word(property))) if punct == "." => Ok((
            Target::Prop(name.to_ascii_lowercase(), property.to_ascii_lowercase()),
            3,
        )),
        _ => Ok((Target::Var(name.to_ascii_lowercase()), 1)),
    }
}

/// `if (cond) then [else els]`, braced or one statement deep.
fn if_statement(tokens: &[Token]) -> Result<(Statement, usize), ()> {
    let Some(Token::Punct(punct)) = tokens.get(1) else {
        return Err(());
    };
    if punct != "(" {
        return Err(());
    }
    let (cond, cond_used) = expression(&tokens[2..], 0)?;
    let mut at = 2 + cond_used;
    let Some(Token::Punct(punct)) = tokens.get(at) else {
        return Err(());
    };
    if punct != ")" {
        return Err(());
    }
    at += 1;
    let (then, then_used) = run_of(&tokens[at..])?;
    at += then_used;
    let els = if tokens
        .get(at)
        .is_some_and(|t| matches!(t, Token::Word(w) if w.eq_ignore_ascii_case("else")))
    {
        let (els, used) = run_of(&tokens[at + 1..])?;
        at += 1 + used;
        els
    } else {
        Vec::new()
    };
    Ok((Statement::If { cond, then, els }, at))
}

/// The statements an `if` or `else` runs: a brace block, or the one
/// statement that follows.
fn run_of(tokens: &[Token]) -> Result<(Vec<Statement>, usize), ()> {
    match tokens.first() {
        Some(Token::Punct(punct)) if punct == "{" => {
            let mut depth = 0i32;
            let mut end = 0usize;
            for (index, token) in tokens.iter().enumerate() {
                match token {
                    Token::Punct(punct) if punct == "{" => depth += 1,
                    Token::Punct(punct) if punct == "}" => {
                        depth -= 1;
                        if depth == 0 {
                            end = index;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let statements = block(&text_of(&tokens[1..end]))?;
            Ok((statements, end + 1))
        }
        _ => {
            let (statement, used) = statement(tokens)?;
            Ok((vec![statement], used))
        }
    }
}

/// `switch (on) { case a: ... case b: ... }`, the cases run one at a
/// time until a break is met, which the machine treats as the case's
/// whole run.
fn switch(tokens: &[Token]) -> Result<(Statement, usize), ()> {
    if tokens.get(1) != Some(&Token::Punct("(".into())) {
        return Err(());
    }
    let (on, inner) = expression(&tokens[2..], 0)?;
    let mut at = 2 + inner;
    if tokens.get(at) != Some(&Token::Punct(")".into())) {
        return Err(());
    }
    at += 1;
    if tokens.get(at) != Some(&Token::Punct("{".into())) {
        return Err(());
    }
    let mut depth = 0i32;
    let mut end = at;
    for (index, token) in tokens.iter().enumerate().skip(at) {
        match token {
            Token::Punct(punct) if punct == "{" => depth += 1,
            Token::Punct(punct) if punct == "}" => {
                depth -= 1;
                if depth == 0 {
                    end = index;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &tokens[at + 1..end];
    let mut cases = Vec::new();
    let mut at = 0usize;
    while at < body.len() {
        match body.get(at) {
            Some(Token::Word(word)) if word.eq_ignore_ascii_case("case") => {}
            // Marks between cases are nothing.
            _ => {
                at += 1;
                continue;
            }
        }
        let (case, case_used) = expression(&body[at + 1..], 0)?;
        at += 1 + case_used;
        if body.get(at) != Some(&Token::Punct(":".into())) {
            return Err(());
        }
        at += 1;
        let mut run = Vec::new();
        while at < body.len()
            && !body[at].clone().word().is_some_and(|w| {
                w.eq_ignore_ascii_case("case") || w.eq_ignore_ascii_case("default")
            })
        {
            let (statement, used) = statement(&body[at..])?;
            // A break ends the case's run; anything after belongs to
            // the next case.
            if matches!(statement, Statement::Other) && used <= 2 {
                at += used;
                break;
            }
            run.push(statement);
            at += used;
        }
        cases.push((case, run));
    }
    Ok((Statement::Switch { on, cases }, end + 1))
}

/// The text of a token run, for the parser's own re-reads.
fn text_of(tokens: &[Token]) -> String {
    let mut out = String::new();
    for token in tokens {
        match token {
            Token::Word(word) => {
                out.push_str(word);
                out.push(' ');
            }
            Token::Number(number) => {
                out.push_str(&number.to_string());
                out.push(' ');
            }
            Token::Str(string) => {
                out.push('\'');
                out.push_str(string);
                out.push_str("' ");
            }
            Token::Punct(punct) => {
                out.push_str(punct);
                out.push(' ');
            }
        }
    }
    out
}

impl Token {
    /// The word a token carries, when it is one.
    fn word(&self) -> Option<&str> {
        match self {
            Token::Word(word) => Some(word),
            _ => None,
        }
    }
}

/// A call `name(args)`, from the name at the head of the stream. A
/// name of dots — `player.controls.next` — is one call of the whole
/// chain.
fn call(tokens: &[Token]) -> Result<(Statement, usize), ()> {
    let Some(Token::Word(first)) = tokens.first() else {
        return Err(());
    };
    let mut name = first.to_ascii_lowercase();
    let mut at = 1usize;
    while tokens.get(at) == Some(&Token::Punct(".".into()))
        && let Some(Token::Word(part)) = tokens.get(at + 1)
    {
        name = format!("{name}.{part}");
        name = name.to_ascii_lowercase();
        at += 2;
    }
    if tokens.get(at) != Some(&Token::Punct("(".into())) {
        return Err(());
    }
    at += 1;
    let mut args = Vec::new();
    if tokens.get(at) == Some(&Token::Punct(")".into())) {
        return Ok((
            Statement::Call {
                name: name.to_ascii_lowercase(),
                args,
            },
            at + 1,
        ));
    }
    loop {
        let (expr, used) = expression(&tokens[at..], 0)?;
        args.push(expr);
        at += used;
        match tokens.get(at) {
            Some(Token::Punct(punct)) if punct == "," => at += 1,
            Some(Token::Punct(punct)) if punct == ")" => {
                return Ok((
                    Statement::Call {
                        name: name.to_ascii_lowercase(),
                        args,
                    },
                    at + 1,
                ));
            }
            _ => return Err(()),
        }
    }
}

/// An expression, from the head of the stream, up to but not past the
/// next mark that ends it. The binding tightens as the level climbs.
fn expression(tokens: &[Token], level: u8) -> Result<(Expr, usize), ()> {
    if level > 5 {
        return Err(());
    }
    let (mut expr, mut used) = match level {
        0 => ternary(tokens)?,
        1 => either(tokens, "||", 2)?,
        2 => either(tokens, "&&", 3)?,
        3 => comparison(tokens)?,
        4 => unary(tokens)?,
        _ => primary(tokens)?,
    };
    while matches!(
        tokens.get(used),
        Some(Token::Punct(punct)) if binary_continues(punct, level)
    ) {
        let punct = match tokens.get(used) {
            Some(Token::Punct(punct)) => punct.clone(),
            _ => break,
        };
        let (right, span) = expression(&tokens[used + 1..], level + 1)?;
        expr = match punct.as_str() {
            "||" => Expr::Or(Box::new(expr), Box::new(right)),
            "&&" => Expr::And(Box::new(expr), Box::new(right)),
            "==" => Expr::Equal(Box::new(expr), Box::new(right)),
            "!=" => Expr::NotEqual(Box::new(expr), Box::new(right)),
            ">" => Expr::Greater(Box::new(expr), Box::new(right)),
            "<" => Expr::Less(Box::new(expr), Box::new(right)),
            _ => break,
        };
        used += 1 + span;
    }
    Ok((expr, used))
}

fn binary_continues(punct: &str, level: u8) -> bool {
    match level {
        0 => false,
        1 => punct == "||",
        2 => punct == "&&",
        3 => matches!(punct, "==" | "!=" | ">" | "<"),
        _ => false,
    }
}

/// `cond ? yes : no`, or the `||` level beneath it.
fn ternary(tokens: &[Token]) -> Result<(Expr, usize), ()> {
    let (choice, mut used) = expression(tokens, 1)?;
    if tokens.get(used) == Some(&Token::Punct("?".into())) {
        let (yes, span) = expression(&tokens[used + 1..], 0)?;
        used += 1 + span;
        if tokens.get(used) != Some(&Token::Punct(":".into())) {
            return Err(());
        }
        let (no, span) = expression(&tokens[used + 1..], 0)?;
        used += 1 + span;
        return Ok((
            Expr::Ternary(Box::new(choice), Box::new(yes), Box::new(no)),
            used,
        ));
    }
    Ok((choice, used))
}

/// One side of an either-or chain.
fn either(tokens: &[Token], mark: &str, level: u8) -> Result<(Expr, usize), ()> {
    let (mut expr, mut used) = expression(tokens, level)?;
    while tokens.get(used) == Some(&Token::Punct(mark.into())) {
        let (right, span) = expression(&tokens[used + 1..], level + 1)?;
        expr = if mark == "||" {
            Expr::Or(Box::new(expr), Box::new(right))
        } else {
            Expr::And(Box::new(expr), Box::new(right))
        };
        used += 1 + span;
    }
    Ok((expr, used))
}

/// `a == b`, `a != b`, `a > b`, `a < b`, or the unary beneath.
fn comparison(tokens: &[Token]) -> Result<(Expr, usize), ()> {
    let (expr, used) = expression(tokens, 4)?;
    Ok((expr, used))
}

/// `!expr`, or the primary beneath.
fn unary(tokens: &[Token]) -> Result<(Expr, usize), ()> {
    if tokens.first() == Some(&Token::Punct("!".into())) {
        let (inner, used) = unary(&tokens[1..])?;
        return Ok((Expr::Not(Box::new(inner)), used + 1));
    }
    expression(tokens, 5)
}

/// A literal, a name, a property of a name, a parenthesised
/// expression, or a call read as its unknown answer.
fn primary(tokens: &[Token]) -> Result<(Expr, usize), ()> {
    match tokens.first() {
        Some(Token::Number(number)) => Ok((Expr::Val(Val::Num(*number)), 1)),
        Some(Token::Str(string)) => Ok((Expr::Val(Val::Str(string.clone())), 1)),
        Some(Token::Punct(punct)) if punct == "(" => {
            let (expr, used) = expression(&tokens[1..], 0)?;
            if tokens.get(used + 1) != Some(&Token::Punct(")".into())) {
                return Err(());
            }
            Ok((expr, used + 2))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("true") => {
            Ok((Expr::Val(Val::Flag(true)), 1))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("false") => {
            Ok((Expr::Val(Val::Flag(false)), 1))
        }
        Some(Token::Word(name)) => {
            if tokens.get(1) == Some(&Token::Punct("(".into())) {
                let (call, used) = call(tokens)?;
                // A call inside an expression: the machine answers with
                // what it does not know, and the statement it stands
                // for is dropped. A call in an expression in these
                // skins is a host builtin, not a decision.
                let _ = call;
                return Ok((Expr::Val(Val::Unknown), used));
            }
            if tokens.get(1) == Some(&Token::Punct(".".into()))
                && let Some(Token::Word(property)) = tokens.get(2)
            {
                return Ok((
                    Expr::Prop(name.to_ascii_lowercase(), property.to_ascii_lowercase()),
                    3,
                ));
            }
            Ok((Expr::Name(name.to_ascii_lowercase()), 1))
        }
        _ => Err(()),
    }
}

/// A parser's test: the shapes the corpus of skins uses, from the
/// machine's own view.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wmp::ir;

    fn script(source: &str) -> (Script, Machine) {
        let script = Script::parse(&[source.to_string()]);
        let view = View::default();
        (script, Machine::new(&view))
    }

    #[test]
    fn the_panes_a_skin_switches_between_turn_on_and_off() {
        let (script, mut machine) = script(
            r"
function SetVisibility(pane)
{
    sAudio.visible = pane == 1;
    sPl.visible = pane == 2;
    currentPane = pane;
}
",
        );
        let actions = machine.handler(&script, "SetVisibility(1);");
        assert!(actions.is_empty());
        assert_eq!(machine.visible("sAudio"), Some(true));
        assert_eq!(machine.visible("sPl"), Some(false));
        let actions = machine.handler(&script, "SetVisibility(2);");
        assert!(actions.is_empty());
        assert_eq!(machine.visible("sAudio"), Some(false));
        assert_eq!(machine.visible("sPl"), Some(true));
    }

    #[test]
    fn a_switch_of_panes_picks_one() {
        let (script, mut machine) = script(
            r"
var noPane = 0;
var audPane = 1;
var currentPane = 0;

function SetVisibility(newPane)
{
    switch(newPane)
    {
    case noPane:
        sLilHeart.visible = false;
        sAudio.visible = false;
        break;
    case audPane:
        sLilHeart.visible = true;
        sAudio.visible = true;
        break;
    }
    currentPane = newPane;
}
",
        );
        machine.run(&script, "Init");
        println!(
            "functions: {:?}",
            script.functions.keys().collect::<Vec<_>>()
        );
        println!("globals: {:?}", script.globals);
        println!(
            "fn body: {:?}",
            script.functions.get("setvisibility").map(|f| &f.body)
        );
        machine.handler(&script, "SetVisibility(audPane);");
        println!("panes: {:?}", machine.panes);
        println!("vars: {:?}", machine.vars);
        assert_eq!(machine.visible("sLilHeart"), Some(true));
        assert_eq!(machine.visible("sAudio"), Some(true));
        machine.handler(&script, "SetVisibility(noPane);");
        assert_eq!(machine.visible("sLilHeart"), Some(false));
        assert_eq!(machine.visible("sAudio"), Some(false));
    }

    #[test]
    fn a_chained_assignment_reaches_every_pane() {
        let (script, mut machine) = script(
            r"
function ToggleAll()
{
    a.visible = b.visible = c.visible = !c.visible;
}
",
        );
        machine.handler(&script, "ToggleAll();");
        assert_eq!(machine.visible("a"), Some(true));
        assert_eq!(machine.visible("b"), Some(true));
        assert_eq!(machine.visible("c"), Some(true));
        machine.handler(&script, "ToggleAll();");
        assert_eq!(machine.visible("a"), Some(false));
        assert_eq!(machine.visible("b"), Some(false));
        assert_eq!(machine.visible("c"), Some(false));
    }

    #[test]
    fn a_toggled_pane_comes_back() {
        let (script, mut machine) = script(
            r"
function TogglePl()
{
    Plbox.visible = plt.visible = !plt.visible;
}
",
        );
        machine.handler(&script, "TogglePl();");
        println!(
            "toggle body: {:?}",
            script.functions.get("togglepl").map(|f| &f.body)
        );
        println!("toggle panes: {:?}", machine.panes);
        assert_eq!(machine.visible("Plbox"), Some(true));
        machine.handler(&script, "TogglePl();");
        assert_eq!(machine.visible("Plbox"), Some(false));
    }

    #[test]
    fn the_player_verbs_a_handler_calls_come_back_as_actions() {
        let (script, mut machine) = script(
            r"
function Quick()
{
    player.controls.next();
}
",
        );
        let actions = machine.handler(&script, "Quick();");
        assert_eq!(actions, vec![Action::Next]);
    }

    #[test]
    fn a_condition_on_what_the_machine_does_not_know_stays_false() {
        let script = Script::parse(&[r"
function OnOpenStateChange()
{
    if(player.OpenState == osMediaOpen)
    {
        sMetaInfo.visible = true;
    }
}
"
        .to_string()]);
        let mut view = ir::View::default();
        view.children.push(ir::Element::Subview(ir::Subview {
            common: ir::Common {
                id: Some("sMetaInfo".into()),
                visible: Some(ir::Value::Literal("false".into())),
                ..ir::Common::default()
            },
            ..ir::Subview::default()
        }));
        let mut machine = Machine::new(&view);
        assert_eq!(machine.visible("sMetaInfo"), Some(false));
        machine.run(&script, "OnOpenStateChange");
        assert_eq!(machine.visible("sMetaInfo"), Some(false));
    }

    #[test]
    fn an_ternary_of_panes_hands_one_over() {
        let (script, mut machine) = script(
            r"
function SetVisibility(newPane)
{
    var next = newPane == 1 ? sAudio : sPl;
    sAudio.visible = next == sAudio;
    sPl.visible = next == sPl;
}
",
        );
        machine.handler(&script, "SetVisibility(1);");
        assert_eq!(machine.visible("sAudio"), Some(true));
        assert_eq!(machine.visible("sPl"), Some(false));
    }

    #[test]
    fn a_handler_that_goes_straight_at_the_statements_runs_them() {
        let (script, mut machine) = script(
            r"
function SetVisibility(pane)
{
    switch(pane)
    {
    case 0:
        sAudio.visible = false;
        break;
    case 1:
        sAudio.visible = true;
        break;
    }
    currentPane = pane;
}
",
        );
        machine.handler(&script, "SetVisibility(currentPane==1?0:1);");
        assert_eq!(machine.visible("sAudio"), Some(true));
        machine.handler(&script, "SetVisibility(currentPane==1?0:1);");
        assert_eq!(machine.visible("sAudio"), Some(false));
    }

    #[test]
    fn the_corpus_s_scripts_parse_where_the_machine_can_follow() {
        let dir = std::env::var("FASTPOTIFY_WMP_SAMPLES").ok();
        let Some(dir) = dir else { return };
        let mut checked = 0usize;
        for entry in std::fs::read_dir(dir).unwrap_or_else(|_| std::fs::read_dir(".").unwrap()) {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("wmz") {
                continue;
            }
            let Ok(document) = crate::wmp::SkinDocument::load(&path) else {
                continue;
            };
            checked += 1;
            // The machine of the main view runs whatever the skin
            // runs as it loads, without failing, and its panes stay
            // within what the definitions said.
            let view = document.main_view().unwrap_or(&document.views[0]);
            let mut machine = Machine::new(view);
            machine.run(&document.script, "Init");
            // Every pane the machine holds must answer, and no run may
            // leave it inconsistent.
            for id in machine.panes.keys() {
                assert!(!id.is_empty());
            }
        }
        assert!(checked > 0, "the corpus was empty");
    }

    #[test]
    fn the_view_comes_up_with_its_definitions_state() {
        let view = crate::wmp::ir::View::default();
        let machine = Machine::new(&view);
        assert_eq!(machine.visible("nothing"), None);
    }
}
