//! GUI language preferences and translations. Project data and CLI output stay unchanged.

use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "en")]
    English,
}

impl Language {
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "ru" => Some(Self::Russian),
            "en" => Some(Self::English),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Russian => "ru",
            Self::English => "en",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Russian => "Русский",
            Self::English => "English",
        }
    }

    pub fn text(self, source: &str) -> &str {
        if self == Self::Russian {
            catalog().get(source).map(String::as_str).unwrap_or(source)
        } else {
            source
        }
    }

    /// Translate a fully formatted status or diagnostic without modifying its data.
    pub fn message<'a>(self, source: &'a str) -> Cow<'a, str> {
        if self == Self::English {
            return Cow::Borrowed(source);
        }
        // This one argument is a list of application diagnostics, not user data.
        if let Some(reasons) = source
            .strip_prefix("SVG contains unsupported ")
            .and_then(|value| {
                value.strip_suffix("; convert text/strokes to flat solid outlines first")
            })
        {
            let mut reasons = reasons.to_owned();
            for reason in [
                "SVG geometry complexity limit",
                "group opacity or blend mode",
                "clipping, masks or filters",
                "partial opacity",
                "combined fill and stroke",
                "gradient or pattern fill",
                "stroke-only paths (convert stroke to outline)",
                "SVG exceeds 100,000 flattened nodes",
                "nonzero path exceeds 256 subpaths",
                "text or embedded raster image",
            ] {
                reasons = reasons.replace(reason, self.text(reason));
            }
            return Cow::Owned(self.text("SVG contains unsupported {}; convert text/strokes to flat solid outlines first").replace("{}", &reasons));
        }
        if let Some(translated) = translate_message(source) {
            return translated;
        }
        // The core may prefix a diagnostic with an arbitrary user object name.
        // Only translate a recognized complete suffix; never translate that name.
        for (index, _) in source.match_indices(": ") {
            if let Some(translated) = translate_message(&source[index + 2..]) {
                return Cow::Owned(format!("{}{}", &source[..index + 2], translated));
            }
        }
        Cow::Borrowed(source)
    }

    pub fn load() -> Self {
        preferences_path()
            .map(|path| Self::load_at(&path))
            .unwrap_or_default()
    }

    fn load_at(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.get("language").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    pub fn save(self) -> Result<(), String> {
        let path =
            preferences_path().ok_or_else(|| "Cannot locate the preferences folder".to_owned())?;
        self.save_at(&path)
    }

    fn save_at(self, path: &Path) -> Result<(), String> {
        let mut preferences = match std::fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes)
                    .map_err(|error| format!("Cannot read language preferences: {error}"))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
            Err(error) => return Err(format!("Cannot read language preferences: {error}")),
        };
        preferences.insert(
            "language".into(),
            serde_json::Value::String(self.code().into()),
        );
        let bytes = serde_json::to_vec_pretty(&preferences).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(path.parent().expect("preferences have a parent"))
            .map_err(|error| format!("Cannot save language preferences: {error}"))?;
        std::fs::write(path, bytes)
            .map_err(|error| format!("Cannot save language preferences: {error}"))
    }
}

fn preferences_path() -> Option<PathBuf> {
    let nonempty_env = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    nonempty_env("APPDATA")
        .or_else(|| nonempty_env("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| nonempty_env("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|root| root.join("EmbroideryLab").join("preferences.json"))
}

fn catalog() -> &'static BTreeMap<String, String> {
    static CATALOG: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("locales/ru.json"))
            .expect("the bundled Russian catalog must be valid JSON")
    })
}

#[derive(Debug)]
enum Part {
    Literal(String),
    Capture(String),
}

/// Parse Rust's ordinary format placeholders and escaped braces. Nested dynamic
/// format specifications are deliberately unsupported, and such keys are skipped.
fn parse_template(template: &str) -> Option<Vec<Part>> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    let mut automatic_index = 0;
    while let Some(character) = chars.next() {
        match character {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                if !literal.is_empty() {
                    parts.push(Part::Literal(std::mem::take(&mut literal)));
                }
                let mut placeholder = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        '{' => return None,
                        value => placeholder.push(value),
                    }
                }
                let identity = placeholder.split(':').next()?.trim();
                let identity = if identity.is_empty() {
                    let identity = automatic_index.to_string();
                    automatic_index += 1;
                    identity
                } else {
                    identity.to_owned()
                };
                parts.push(Part::Capture(identity));
            }
            '}' => return None,
            value => literal.push(value),
        }
    }
    if !literal.is_empty() {
        parts.push(Part::Literal(literal));
    }
    Some(parts)
}

#[derive(Debug)]
enum Token {
    Literal(String),
    Capture(usize),
}

#[derive(Debug)]
struct Pattern {
    source: Vec<Token>,
    translated: Vec<Token>,
    slots: usize,
    specificity: usize,
}

impl Pattern {
    fn compile(source: &str, translated: &str) -> Option<Self> {
        let source = parse_template(source)?;
        let mut identities = Vec::new();
        let mut specificity = 0;
        let mut letters = 0;
        let mut previous_capture = false;
        let mut tokens = Vec::new();
        for part in source {
            match part {
                Part::Literal(value) => {
                    specificity += value.len();
                    letters += value
                        .chars()
                        .filter(|character| character.is_alphabetic())
                        .count();
                    previous_capture = false;
                    tokens.push(Token::Literal(value));
                }
                Part::Capture(identity) => {
                    if previous_capture {
                        return None;
                    }
                    previous_capture = true;
                    let slot = identities
                        .iter()
                        .position(|value| value == &identity)
                        .unwrap_or_else(|| {
                            identities.push(identity);
                            identities.len() - 1
                        });
                    tokens.push(Token::Capture(slot));
                }
            }
        }
        // A pattern such as "{}: {e}" must never claim arbitrary diagnostics.
        if identities.is_empty() || identities.len() > 16 || letters < 3 {
            return None;
        }
        let translated = parse_template(translated)?
            .into_iter()
            .map(|part| match part {
                Part::Literal(value) => Some(Token::Literal(value)),
                Part::Capture(identity) => identities
                    .iter()
                    .position(|value| value == &identity)
                    .map(Token::Capture),
            })
            .collect::<Option<Vec<_>>>()?;
        if (0..identities.len()).any(|slot| {
            !translated
                .iter()
                .any(|token| matches!(token, Token::Capture(value) if *value == slot))
        }) {
            return None;
        }
        Some(Self {
            source: tokens,
            translated,
            slots: identities.len(),
            specificity,
        })
    }

    fn translate(&self, source: &str) -> Option<String> {
        if source.len() > 65_536 {
            return None;
        }
        let mut captures = vec![None; self.slots];
        let mut budget = 2048;
        if !self.matches(0, source, &mut captures, &mut budget) {
            return None;
        }
        let mut result = String::new();
        for token in &self.translated {
            match token {
                Token::Literal(value) => result.push_str(value),
                Token::Capture(slot) => result.push_str(captures[*slot]?),
            }
        }
        Some(result)
    }

    fn matches<'a>(
        &self,
        index: usize,
        remaining: &'a str,
        captures: &mut [Option<&'a str>],
        budget: &mut usize,
    ) -> bool {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        let Some(token) = self.source.get(index) else {
            return remaining.is_empty();
        };
        match token {
            Token::Literal(value) => remaining
                .strip_prefix(value)
                .is_some_and(|rest| self.matches(index + 1, rest, captures, budget)),
            Token::Capture(slot) => {
                let previous = captures[*slot];
                let mut try_capture = |length| {
                    let value = &remaining[..length];
                    if previous.is_some_and(|previous| previous != value) {
                        return false;
                    }
                    captures[*slot] = Some(value);
                    if self.matches(index + 1, &remaining[length..], captures, budget) {
                        return true;
                    }
                    captures[*slot] = previous;
                    false
                };
                if let Some(Token::Literal(delimiter)) = self.source.get(index + 1) {
                    remaining
                        .match_indices(delimiter)
                        .any(|(position, _)| try_capture(position))
                } else {
                    try_capture(remaining.len())
                }
            }
        }
    }
}

fn patterns() -> &'static [Pattern] {
    static PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let mut patterns: Vec<_> = catalog()
            .iter()
            .filter_map(|(source, translated)| Pattern::compile(source, translated))
            .collect();
        patterns.sort_by_key(|pattern| std::cmp::Reverse(pattern.specificity));
        patterns
    })
}

fn translate_message(source: &str) -> Option<Cow<'_, str>> {
    if let Some(translated) = catalog().get(source) {
        return Some(Cow::Borrowed(translated));
    }
    patterns()
        .iter()
        .find_map(|pattern| pattern.translate(source).map(Cow::Owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_and_serialization_are_stable() {
        assert_eq!(Language::default(), Language::Russian);
        for language in [Language::Russian, Language::English] {
            assert_eq!(Language::from_code(language.code()), Some(language));
            assert_eq!(
                serde_json::from_str::<Language>(&format!("\"{}\"", language.code())).unwrap(),
                language
            );
        }
        assert_eq!(Language::from_code("unknown"), None);
    }

    #[test]
    fn formatted_paths_remain_verbatim() {
        let source = r"Saved C:\Работа\Saved {} — рисунок.elab";
        assert_eq!(
            Language::Russian.message(source),
            r"Сохранено: C:\Работа\Saved {} — рисунок.elab"
        );
        assert_eq!(Language::English.message(source), source);
    }

    #[test]
    fn count_messages_work_for_russian_plural_forms() {
        for count in [0, 1, 2, 5, 21] {
            assert_eq!(
                Language::Russian.message(&format!("{count} needle stitches · {count} jumps")),
                format!("Стежков: {count} · переходов: {count}")
            );
        }
    }

    #[test]
    fn names_formatting_and_escaped_braces_are_preserved() {
        let pattern = Pattern::compile(
            "Grid {{data}} {scale:.1} px/mm: {name}",
            "{name}: сетка {{data}} {scale:.1} пкс/мм",
        )
        .unwrap();
        assert_eq!(
            pattern.translate("Grid {data} 5.0 px/mm: Объект: 1"),
            Some("Объект: 1: сетка {data} 5.0 пкс/мм".into())
        );
        let positional =
            Pattern::compile("Item {} has {:.2} units", "Значение {1:.2} для {0}").unwrap();
        assert_eq!(
            positional.translate("Item A has 1.20 units"),
            Some("Значение 1.20 для A".into())
        );
    }

    #[test]
    fn unknown_or_broad_diagnostics_are_unchanged() {
        let source = "Имя: unknown error: unsupported external failure";
        assert_eq!(Language::Russian.message(source), source);
        assert!(Pattern::compile("{}: {e}", "{e}: {}").is_none());
        assert!(Pattern::compile("{}{}", "{}{}").is_none());
        assert!(Pattern::compile("Value {name:{width}}", "{name}").is_none());
        let pattern = Pattern::compile("Saved {}", "Сохранено: {}").unwrap();
        assert!(pattern.translate("prefix Saved path").is_none());
    }

    #[test]
    fn recognized_object_diagnostic_preserves_its_name() {
        let source =
            "Мой объект {}: Place at least three points before completing a closed contour.";
        assert_eq!(
            Language::Russian.message(source),
            "Мой объект {}: Добавьте не менее трёх точек, чтобы замкнуть контур."
        );
    }

    #[test]
    fn unsupported_svg_reasons_are_translated_as_diagnostics() {
        assert_eq!(
            Language::Russian.message("SVG contains unsupported group opacity or blend mode, clipping, masks or filters; convert text/strokes to flat solid outlines first"),
            "В SVG есть неподдерживаемые элементы: непрозрачность группы или режим наложения, обтравка, маски или фильтры; сначала преобразуйте текст и обводки в плоские контуры со сплошной заливкой"
        );
    }

    #[test]
    fn core_diagnostics_preserve_hex_numbers_and_object_names() {
        assert_eq!(
            Language::Russian.message("Unsupported DSB control 0x9a at record 21"),
            "Неподдерживаемый код DSB 0x9a в записи 21"
        );
        assert_eq!(
            Language::Russian.message("Рисунок: слой 2: Satin rail contains invalid coordinates"),
            "Рисунок: слой 2: В направляющей глади есть недопустимые координаты"
        );
    }

    #[test]
    fn preferences_roundtrip_preserves_unrelated_fields() {
        let root = std::env::temp_dir().join(format!("embroidery-language-{}", std::process::id()));
        let path = root.join("Русский").join("preferences.json");
        assert_eq!(Language::load_at(&path), Language::Russian);
        Language::English.save_at(&path).unwrap();
        assert_eq!(Language::load_at(&path), Language::English);
        std::fs::write(
            &path,
            br#"{"language":"en","recent":["project.elab"],"window":{"width":1360}}"#,
        )
        .unwrap();
        Language::Russian.save_at(&path).unwrap();
        assert_eq!(Language::load_at(&path), Language::Russian);
        let preferences: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(preferences["recent"], serde_json::json!(["project.elab"]));
        assert_eq!(preferences["window"]["width"], 1360);
        std::fs::write(&path, b"invalid JSON").unwrap();
        assert_eq!(Language::load_at(&path), Language::Russian);
        assert!(Language::English.save_at(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid JSON");
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn every_catalog_template_preserves_dynamic_data() {
        for (source, translated) in catalog() {
            let parts = parse_template(source).unwrap();
            let literal_letters: usize = parts
                .iter()
                .filter_map(|part| match part {
                    Part::Literal(value) => Some(
                        value
                            .chars()
                            .filter(|character| character.is_alphabetic())
                            .count(),
                    ),
                    _ => None,
                })
                .sum();
            if literal_letters < 3 || !parts.iter().any(|part| matches!(part, Part::Capture(_))) {
                continue;
            }
            let pattern = Pattern::compile(source, translated).expect(source);
            let sentinels: Vec<_> = (0..pattern.slots)
                .map(|slot| format!("Путь_{slot}_значение"))
                .collect();
            let render = |tokens: &[Token]| {
                tokens
                    .iter()
                    .map(|token| match token {
                        Token::Literal(value) => value.as_str(),
                        Token::Capture(slot) => sentinels[*slot].as_str(),
                    })
                    .collect::<String>()
            };
            assert_eq!(
                pattern.translate(&render(&pattern.source)),
                Some(render(&pattern.translated)),
                "{source}"
            );
        }
    }
}
