//! 輕量 persona 模板展開器。
//!
//! 在 system prompt 被注入前，將 `{{expr}}` 佔位符替換為執行時的動態值。
//! 無法辨識的運算式會保持原樣（`{{unknown}}`），確保向前相容。
//!
//! # 支援的運算式
//!
//! | 語法                  | 展開結果                          |
//! |-----------------------|-----------------------------------|
//! | `{{today()}}`         | 今日日期，格式 `YYYY-MM-DD`       |
//! | `{{now()}}`           | 現在時間，格式 `YYYY-MM-DD HH:MM:SS` |
//! | `{{cwd()}}`           | 目前工作目錄絕對路徑              |
//! | `{{env("VAR_NAME")}}` | 環境變數值（不存在則為空字串）    |

use chrono::Local;
use std::path::Path;

/// 展開 `template` 字串中所有 `{{...}}` 佔位符。
/// `cwd` 為呼叫時的工作目錄，用於 `{{cwd()}}` 展開。
pub fn expand(template: &str, cwd: &Path) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find("{{") {
        // 輸出佔位符之前的純文字
        result.push_str(&rest[..open]);
        rest = &rest[open + 2..];

        match rest.find("}}") {
            Some(close) => {
                let expr = rest[..close].trim();
                result.push_str(&eval(expr, cwd));
                rest = &rest[close + 2..];
            }
            None => {
                // 未閉合的 `{{`，視為純文字輸出
                result.push_str("{{");
            }
        }
    }

    // 佔位符之後的剩餘純文字
    result.push_str(rest);
    result
}

/// 計算單一運算式的展開值。
fn eval(expr: &str, cwd: &Path) -> String {
    match expr {
        "today()" => Local::now().format("%Y-%m-%d").to_string(),
        "now()" => Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "cwd()" => cwd.to_string_lossy().into_owned(),
        _ if is_env_call(expr) => eval_env(expr),
        // 無法辨識的運算式：原樣保留
        _ => format!("{{{{{expr}}}}}"),
    }
}

/// 判斷是否為 `env("VAR")` 形式。
fn is_env_call(expr: &str) -> bool {
    expr.starts_with("env(") && expr.ends_with(')')
}

/// 展開 `env("VAR_NAME")`，找不到變數時回傳空字串。
fn eval_env(expr: &str) -> String {
    // expr 形如: env("VAR_NAME")
    let inner = expr["env(".len()..expr.len() - 1].trim();

    let var_name = if (inner.starts_with('"') && inner.ends_with('"'))
        || (inner.starts_with('\'') && inner.ends_with('\''))
    {
        &inner[1..inner.len() - 1]
    } else {
        return format!("{{{{{expr}}}}}"); // 格式不符，保留原樣
    };

    std::env::var(var_name).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cwd() -> PathBuf {
        PathBuf::from("/home/user/project")
    }

    #[test]
    fn no_placeholders() {
        assert_eq!(expand("Hello world", &cwd()), "Hello world");
    }

    #[test]
    fn today_placeholder() {
        let result = expand("Today is {{today()}}.", &cwd());
        // 格式驗證：YYYY-MM-DD
        assert!(result.starts_with("Today is "));
        let date_part = result
            .strip_prefix("Today is ")
            .unwrap()
            .strip_suffix('.')
            .unwrap();
        assert_eq!(date_part.len(), 10);
        assert_eq!(date_part.chars().nth(4), Some('-'));
        assert_eq!(date_part.chars().nth(7), Some('-'));
    }

    #[test]
    fn now_placeholder() {
        let result = expand("Time: {{now()}}", &cwd());
        assert!(result.starts_with("Time: "));
        let ts = result.strip_prefix("Time: ").unwrap();
        // YYYY-MM-DD HH:MM:SS = 19 chars
        assert_eq!(ts.len(), 19);
    }

    #[test]
    fn cwd_placeholder() {
        let result = expand("Working in {{cwd()}} now.", &cwd());
        assert_eq!(result, "Working in /home/user/project now.");
    }

    #[test]
    fn env_placeholder_existing() {
        // SAFETY: single-threaded test binary; no concurrent env access.
        unsafe { std::env::set_var("CCODE_TEST_VAR", "hello") };
        let result = expand(r#"Val: {{env("CCODE_TEST_VAR")}}"#, &cwd());
        assert_eq!(result, "Val: hello");
        unsafe { std::env::remove_var("CCODE_TEST_VAR") };
    }

    #[test]
    fn env_placeholder_missing() {
        // SAFETY: single-threaded test binary; no concurrent env access.
        unsafe { std::env::remove_var("CCODE_NONEXISTENT_ZZZZZ") };
        let result = expand(r#"Val: {{env("CCODE_NONEXISTENT_ZZZZZ")}}"#, &cwd());
        assert_eq!(result, "Val: ");
    }

    #[test]
    fn unknown_expression_preserved() {
        let result = expand("{{unknown_func()}}", &cwd());
        assert_eq!(result, "{{unknown_func()}}");
    }

    #[test]
    fn unclosed_brace_preserved() {
        let result = expand("start {{ no close", &cwd());
        assert_eq!(result, "start {{ no close");
    }

    #[test]
    fn multiple_placeholders() {
        let tpl = "Date: {{today()}}, Dir: {{cwd()}}";
        let result = expand(tpl, &cwd());
        assert!(result.contains("Dir: /home/user/project"));
        assert!(result.starts_with("Date: "));
    }

    #[test]
    fn multiline_template() {
        let tpl = "Line 1: {{today()}}\nLine 2: {{cwd()}}\nLine 3: end";
        let result = expand(tpl, &cwd());
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("Line 1: "));
        assert_eq!(lines[1], "Line 2: /home/user/project");
        assert_eq!(lines[2], "Line 3: end");
    }
}
