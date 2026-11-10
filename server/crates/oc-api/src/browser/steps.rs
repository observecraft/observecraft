//! Browser test steps, in Datadog's browser step model (`type`, `params.element.userLocator`, `allowFailure`,
//! `isCritical`, `timeout`): what a step does and the checks a definition passes before it is saved.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::routes::synthetics_settings as vars;

pub const MAX_STEPS: usize = 100;
/// Seconds a step waits for its element or assertion, at most (oc-browser's default is 60).
pub const MAX_STEP_TIMEOUT: u32 = 300;
/// A `wait` step, at most (seconds).
pub const MAX_WAIT: u32 = 300;
const MAX_VALUE_CHARS: usize = 10_000;
const MAX_LOCATOR_CHARS: usize = 2000;
const MAX_CODE_CHARS: usize = 20_000;
const MAX_LOCATORS: usize = 10;
/// Keys a `pressKey` step may press (besides one printable character).
pub const KEYS: [&str; 16] = ["Enter", "Tab", "Escape", "Backspace", "Delete", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown", "Space", "Insert", "F5"];
pub const MODIFIERS: [&str; 4] = ["Alt", "Control", "Meta", "Shift"];

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
#[schema(as = BrowserStepType)]
pub enum StepType {
    GoToUrl,
    Click,
    TypeText,
    SelectOption,
    Hover,
    PressKey,
    Scroll,
    Refresh,
    Wait,
    AssertElementPresent,
    AssertElementContent,
    AssertElementAttribute,
    AssertPageContains,
    AssertPageLacks,
    AssertCurrentUrl,
    AssertFromJavascript,
    ExtractVariable,
    ExtractFromJavascript,
}

impl StepType {
    /// The step acts on an element (`params.element`).
    pub fn needs_element(self) -> bool {
        use StepType::*;
        matches!(self, Click | TypeText | SelectOption | Hover | AssertElementPresent | AssertElementContent | AssertElementAttribute | ExtractVariable)
    }
}

/// How a value is compared (Datadog's `check`).
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
#[schema(as = BrowserStepCheck)]
pub enum Check {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    NotStartsWith,
    Greater,
    Lower,
    GreaterEquals,
    LowerEquals,
    MatchRegex,
    IsEmpty,
    NotIsEmpty,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
#[schema(as = BrowserLocatorType)]
pub enum LocatorType {
    Css,
    Xpath,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Eq, Debug)]
#[schema(as = BrowserLocator)]
pub struct Locator {
    pub r#type: LocatorType,
    pub value: String,
}

/// The element's locators, tried in order; the first that finds a visible element wins.
#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[schema(as = BrowserUserLocator)]
pub struct UserLocator {
    #[serde(default)]
    pub fail_test_on_cannot_locate: bool,
    #[serde(default)]
    pub values: Vec<Locator>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[schema(as = BrowserElement)]
pub struct Element {
    #[serde(default)]
    pub user_locator: UserLocator,
    /// What the element looked like when the step was made (for people reading the test).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[schema(as = BrowserStepVariable)]
pub struct StepVariable {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub secure: bool,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[schema(as = BrowserClickType)]
pub enum ClickType {
    #[default]
    Primary,
    Double,
    Contextual,
}

/// A step's parameters; each type uses some of them (see [`check_step`]).
#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[schema(as = BrowserStepParams)]
pub struct StepParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<Element>,
    /// The URL, text, option, key or expected value; seconds for `wait`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<Check>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_type: Option<ClickType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append_to_content: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable: Option<StepVariable>,
}

impl StepParams {
    /// `value` as text (numbers as written).
    pub fn text(&self) -> String {
        match &self.value {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
#[schema(as = BrowserStep)]
pub struct Step {
    #[serde(default)]
    pub name: String,
    pub r#type: StepType,
    #[serde(default)]
    pub params: StepParams,
    /// A failure doesn't stop the test.
    #[serde(default)]
    pub allow_failure: bool,
    /// With `allowFailure`: the test still fails when this step does.
    #[serde(default)]
    pub is_critical: bool,
    #[serde(default)]
    pub no_screenshot: bool,
    /// Seconds to wait for the element or assertion (default 60).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
}

/// A variable name as extracted steps define it (`ORDER_ID`).
pub fn valid_variable_name(name: &str) -> bool {
    (1..=80).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') && !name.as_bytes()[0].is_ascii_digit()
}

/// An http(s) URL with a host, `{{ VARIABLES }}` allowed.
pub fn check_url(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(&vars::neutralized(raw)).map_err(|_| format!("'{raw}' isn't a full URL, such as https://shop.example.com"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(format!("'{raw}' isn't an http or https URL"));
    }
    Ok(())
}

fn bounded(text: &str, what: &str, max: usize) -> Result<String, String> {
    let t = text.trim();
    if t.chars().count() > max {
        return Err(format!("the {what} is at most {max} characters"));
    }
    Ok(t.to_string())
}

/// The element's description for a step's default name: its first locator.
fn element_label(p: &StepParams) -> String {
    let el = p.element.as_ref();
    el.filter(|e| !e.description.is_empty()).map(|e| e.description.clone()).or_else(|| el.and_then(|e| e.user_locator.values.first()).map(|l| l.value.clone())).unwrap_or_default()
}

fn default_name(s: &Step) -> String {
    let (p, el) = (&s.params, element_label(&s.params));
    match s.r#type {
        StepType::GoToUrl => format!("Navigate to {}", p.text()),
        StepType::Click => format!("Click on {el}"),
        StepType::TypeText => format!("Type text on {el}"),
        StepType::SelectOption => format!("Select option '{}' in {el}", p.text()),
        StepType::Hover => format!("Hover over {el}"),
        StepType::PressKey => format!("Press {}", p.text()),
        StepType::Scroll => "Scroll".into(),
        StepType::Refresh => "Refresh the page".into(),
        StepType::Wait => format!("Wait {} seconds", p.text()),
        StepType::AssertElementPresent => format!("Test {el} is present"),
        StepType::AssertElementContent => format!("Test {el} content"),
        StepType::AssertElementAttribute => format!("Test {el} attribute {}", p.attribute.clone().unwrap_or_default()),
        StepType::AssertPageContains => format!("Test page contains '{}'", p.text()),
        StepType::AssertPageLacks => format!("Test page lacks '{}'", p.text()),
        StepType::AssertCurrentUrl => format!("Test current URL {}", p.text()),
        StepType::AssertFromJavascript => "Test custom JavaScript assertion".into(),
        StepType::ExtractVariable | StepType::ExtractFromJavascript => format!("Extract variable {}", p.variable.as_ref().map(|v| v.name.as_str()).unwrap_or_default()),
    }
}

/// Checks the `check` and expected value of an assertion.
fn check_comparison(p: &mut StepParams) -> Result<(), String> {
    let check = p.check.ok_or("choose a check (equals, contains, …)")?;
    if matches!(check, Check::IsEmpty | Check::NotIsEmpty) {
        p.value = None;
        return Ok(());
    }
    let value = bounded(&p.text(), "expected value", MAX_VALUE_CHARS)?;
    if value.is_empty() {
        return Err("the expected value is required".into());
    }
    if check == Check::MatchRegex && vars::placeholders(&value).is_empty() {
        regex::Regex::new(&value).map_err(|e| format!("invalid regular expression: {e}"))?;
    }
    p.value = Some(value.into());
    Ok(())
}

/// Checks and normalizes one step.
pub fn check_step(s: &mut Step) -> Result<(), String> {
    use StepType::*;
    if s.timeout.is_some_and(|t| !(1..=MAX_STEP_TIMEOUT).contains(&t)) {
        return Err(format!("the timeout is 1–{MAX_STEP_TIMEOUT} seconds"));
    }
    s.name = bounded(&s.name, "name", 300)?;
    let p = &mut s.params;
    if s.r#type.needs_element() {
        let el = p.element.as_mut().ok_or("choose the element it acts on")?;
        el.description = bounded(&el.description, "element description", 300)?;
        let locators = &mut el.user_locator.values;
        for l in locators.iter_mut() {
            l.value = bounded(&l.value, "element locator", MAX_LOCATOR_CHARS)?;
        }
        locators.retain(|l| !l.value.is_empty());
        if locators.is_empty() || locators.len() > MAX_LOCATORS {
            return Err(format!("give the element 1–{MAX_LOCATORS} CSS or XPath locators"));
        }
    } else if !matches!(s.r#type, Scroll | AssertFromJavascript | ExtractFromJavascript) {
        p.element = None;
    }
    match s.r#type {
        GoToUrl => {
            let url = bounded(&p.text(), "URL", 2048)?;
            check_url(&url)?;
            p.value = Some(url.into());
        }
        TypeText => {
            // Text is typed as written (spaces included).
            let text = p.text();
            if text.is_empty() || text.chars().count() > MAX_VALUE_CHARS {
                return Err(format!("the text to type is 1–{MAX_VALUE_CHARS} characters"));
            }
        }
        SelectOption => {
            let option = bounded(&p.text(), "option", 1000)?;
            if option.is_empty() {
                return Err("name the option to select (its value or label)".into());
            }
            p.value = Some(option.into());
        }
        PressKey => {
            let key = p.text();
            if !(KEYS.contains(&key.as_str()) || key.chars().count() == 1) {
                return Err(format!("the key is one character or one of {}", KEYS.join(", ")));
            }
            if let Some(m) = p.modifiers.as_ref().and_then(|m| m.iter().find(|m| !MODIFIERS.contains(&m.as_str()))) {
                return Err(format!("unknown modifier '{m}' (Alt, Control, Meta, Shift)"));
            }
        }
        Wait => {
            let seconds = p.text().trim().parse::<u32>().ok().filter(|s| (1..=MAX_WAIT).contains(s)).ok_or(format!("wait 1–{MAX_WAIT} seconds"))?;
            p.value = Some(seconds.into());
        }
        AssertPageContains | AssertPageLacks => {
            let text = bounded(&p.text(), "text", MAX_VALUE_CHARS)?;
            if text.is_empty() {
                return Err("the text to look for is required".into());
            }
            p.value = Some(text.into());
        }
        AssertCurrentUrl | AssertElementContent => check_comparison(p)?,
        AssertElementAttribute => {
            let attribute = bounded(p.attribute.as_deref().unwrap_or_default(), "attribute", 200)?;
            if attribute.is_empty() {
                return Err("name the attribute to check".into());
            }
            p.attribute = Some(attribute);
            check_comparison(p)?;
        }
        AssertFromJavascript | ExtractFromJavascript => {
            let code = p.code.as_deref().unwrap_or_default();
            if code.trim().is_empty() || code.chars().count() > MAX_CODE_CHARS {
                return Err(format!("the code is 1–{MAX_CODE_CHARS} characters"));
            }
        }
        Click | Hover | AssertElementPresent | Scroll | Refresh | ExtractVariable => {}
    }
    if matches!(s.r#type, ExtractVariable | ExtractFromJavascript) {
        let v = p.variable.get_or_insert_default();
        v.name = v.name.trim().to_string();
        if !valid_variable_name(&v.name) {
            return Err("the variable name is in capitals, digits and underscores (ORDER_ID)".into());
        }
    }
    if s.name.is_empty() {
        s.name = default_name(s);
    }
    Ok(())
}

/// Checks and normalizes a test's steps; the error names the step.
pub fn check_steps(steps: &mut [Step]) -> Result<(), String> {
    if steps.len() > MAX_STEPS {
        return Err(format!("A test has at most {MAX_STEPS} steps"));
    }
    for (i, s) in steps.iter_mut().enumerate() {
        check_step(s).map_err(|e| format!("Step {}: {e}", i + 1))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(v: serde_json::Value) -> Step {
        serde_json::from_value(v).unwrap()
    }

    fn css(selector: &str) -> serde_json::Value {
        json!({"userLocator": {"values": [{"type": "css", "value": selector}]}})
    }

    #[test]
    fn steps_read_datadogs_json() {
        let s = step(json!({"name": "Click on button Checkout", "type": "click", "allowFailure": true, "isCritical": true, "timeout": 10,
            "params": {"element": {"userLocator": {"failTestOnCannotLocate": true, "values": [{"type": "xpath", "value": "//button[@id='checkout']"}]}}}}));
        assert_eq!(s.r#type, StepType::Click);
        assert!(s.allow_failure && s.is_critical);
        assert_eq!(s.timeout, Some(10));
        let el = s.params.element.unwrap();
        assert_eq!(el.user_locator.values[0], Locator { r#type: LocatorType::Xpath, value: "//button[@id='checkout']".into() });
        let wait = step(json!({"type": "wait", "params": {"value": 3}}));
        assert_eq!(wait.params.text(), "3");
        assert_eq!(wait.timeout, None);
    }

    #[test]
    fn valid_steps_pass_and_are_trimmed() {
        let mut steps = vec![
            step(json!({"type": "goToUrl", "params": {"value": " https://shop.example.com/cart "}})),
            step(json!({"type": "typeText", "params": {"element": css(" #email "), "value": "{{ EMAIL }}"}})),
            step(json!({"type": "assertCurrentUrl", "params": {"check": "contains", "value": "/checkout"}})),
            step(json!({"type": "assertPageLacks", "params": {"value": "Error"}})),
            step(json!({"type": "pressKey", "params": {"value": "Enter", "modifiers": ["Shift"]}})),
            step(json!({"type": "pressKey", "params": {"value": "a"}})),
            step(json!({"type": "wait", "params": {"value": 2}})),
            step(json!({"type": "refresh"})),
            step(json!({"type": "scroll", "params": {"x": 0, "y": 400}})),
            step(json!({"type": "extractVariable", "params": {"element": css(".order-id"), "variable": {"name": "ORDER_ID"}}})),
            step(json!({"type": "assertElementContent", "params": {"element": css("h1"), "check": "isEmpty"}})),
            step(json!({"type": "assertElementAttribute", "params": {"element": css("a"), "attribute": "href", "check": "matchRegex", "value": "^/p/\\d+$"}})),
            step(json!({"type": "assertFromJavascript", "params": {"code": "return document.title.length > 0"}})),
        ];
        check_steps(&mut steps).unwrap();
        assert_eq!(steps[0].params.text(), "https://shop.example.com/cart");
        assert_eq!(steps[1].params.element.as_ref().unwrap().user_locator.values[0].value, "#email");
        // Steps get a readable name when they have none.
        assert_eq!(steps[0].name, "Navigate to https://shop.example.com/cart");
        assert_eq!(steps[1].name, "Type text on #email");
    }

    #[test]
    fn invalid_steps_are_refused_with_their_number() {
        let cases = [
            (json!({"type": "click"}), "element"),
            (json!({"type": "click", "params": {"element": {"userLocator": {"values": []}}}}), "element"),
            (json!({"type": "click", "params": {"element": css("  ")}}), "element"),
            (json!({"type": "goToUrl", "params": {"value": "shop.example.com"}}), "URL"),
            (json!({"type": "goToUrl", "params": {"value": "ftp://shop.example.com"}}), "URL"),
            (json!({"type": "typeText", "params": {"element": css("#q")}}), "text"),
            (json!({"type": "wait", "params": {"value": 0}}), "wait"),
            (json!({"type": "wait", "params": {"value": 301}}), "wait"),
            (json!({"type": "wait", "params": {"value": "soon"}}), "wait"),
            (json!({"type": "pressKey", "params": {"value": "Hyper"}}), "key"),
            (json!({"type": "pressKey", "params": {"value": "Enter", "modifiers": ["Fn"]}}), "modifier"),
            (json!({"type": "assertCurrentUrl", "params": {"value": "/x"}}), "check"),
            (json!({"type": "assertCurrentUrl", "params": {"check": "contains"}}), "value"),
            (json!({"type": "assertElementContent", "params": {"element": css("h1"), "check": "matchRegex", "value": "("}}), "regular expression"),
            (json!({"type": "assertElementAttribute", "params": {"element": css("a"), "check": "equals", "value": "x"}}), "attribute"),
            (json!({"type": "assertPageContains", "params": {"value": ""}}), "text"),
            (json!({"type": "extractVariable", "params": {"element": css("b"), "variable": {"name": "order id"}}}), "variable"),
            (json!({"type": "assertFromJavascript", "params": {"code": " "}}), "code"),
            (json!({"type": "selectOption", "params": {"element": css("select")}}), "option"),
            (json!({"type": "refresh", "timeout": 0}), "timeout"),
            (json!({"type": "refresh", "timeout": 301}), "timeout"),
        ];
        for (raw, word) in cases {
            let mut steps = vec![step(json!({"type": "refresh"})), step(raw.clone())];
            let err = check_steps(&mut steps).expect_err(&raw.to_string());
            assert!(err.starts_with("Step 2:") && err.contains(word), "{raw}: {err}");
        }
        let mut many = vec![step(json!({"type": "refresh"})); MAX_STEPS + 1];
        assert!(check_steps(&mut many).is_err());
    }

    #[test]
    fn variable_names_are_upper_snake_case() {
        assert!(valid_variable_name("ORDER_ID") && valid_variable_name("A1"));
        assert!(!valid_variable_name("") && !valid_variable_name("order") && !valid_variable_name("1A") && !valid_variable_name("A-B"));
    }
}
