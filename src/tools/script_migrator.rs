/// Script migration utilities: check and patch legacy .rhai strategy files.
///
/// A script is considered "current" when it has:
///   1. max_token_p <= 0.65 (better risk/reward threshold)
///   2. The edge filter block (debug_edge key present)
use std::path::Path;

#[derive(Debug, serde::Serialize)]
pub struct ScriptStatus {
    pub name: String,
    pub path: String,
    pub outdated: bool,
    pub has_edge_filter: bool,
    pub max_token_p: Option<f64>,
    pub needs_max_token_update: bool,
    pub needs_edge_filter: bool,
}

/// Check all .rhai files in `scripts_dir` and return their status.
pub fn check_scripts(scripts_dir: &Path) -> Vec<ScriptStatus> {
    let mut result = Vec::new();
    let Ok(entries) = std::fs::read_dir(scripts_dir) else { return result; };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rhai") {
            continue;
        }
        let name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let max_token_p = extract_max_token_p(&content);
        let has_edge_filter = content.contains("debug_edge");
        let needs_max_token_update = max_token_p.map(|v| v > 0.65).unwrap_or(false);
        let needs_edge_filter = !has_edge_filter && content.contains("on_candle");

        result.push(ScriptStatus {
            outdated: needs_max_token_update || needs_edge_filter,
            name,
            path: path.to_string_lossy().to_string(),
            has_edge_filter,
            max_token_p,
            needs_max_token_update,
            needs_edge_filter,
        });
    }
    result
}

/// Patch a single script: update max_token_p and inject edge filter if missing.
/// Returns the patched content, or an error string.
pub fn patch_script(content: &str) -> Result<String, String> {
    let mut patched = content.to_string();

    // 1. Update max_token_p to 0.65
    if let Some(new_content) = update_max_token_p(&patched) {
        patched = new_content;
    }

    // 2. Inject edge filter block if missing
    if !patched.contains("debug_edge") {
        patched = inject_edge_filter(&patched)?;
    }

    Ok(patched)
}

// ── Internals ─────────────────────────────────────────────────────────────────

fn extract_max_token_p(content: &str) -> Option<f64> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("let max_token_p") {
            // e.g. "let max_token_p = 0.82;"
            if let Some(eq_pos) = trimmed.find('=') {
                let val_str = trimmed[eq_pos + 1..]
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                if let Ok(v) = val_str.parse::<f64>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn update_max_token_p(content: &str) -> Option<String> {
    let mut changed = false;
    let result = content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("let max_token_p") {
                if let Some(eq_pos) = trimmed.find('=') {
                    let val_str = trimmed[eq_pos + 1..]
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    if let Ok(v) = val_str.parse::<f64>() {
                        if v > 0.65 {
                            changed = true;
                            // Preserve indentation
                            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                            return format!("{}let max_token_p = 0.65;  // updated by migrator (was {:.2})", indent, v);
                        }
                    }
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    if changed { Some(result) } else { None }
}

const EDGE_FILTER_BLOCK: &str = r#"
    // ── Edge filter (injected by script migrator) ─────────────────────────────
    // Maps abs(score) → estimated win probability. Calibrate min_edge with data.
    let abs_score = if score >= 0.0 { score } else { -score };
    let est_prob = if abs_score >= 8.0 { 0.85 }
                   else if abs_score >= 7.0 { 0.80 }
                   else if abs_score >= 6.0 { 0.75 }
                   else if abs_score >= 5.0 { 0.70 }
                   else if abs_score >= 4.0 { 0.65 }
                   else { 0.62 };
    let implied_p = if score > 0.0 { ctx.token_price } else { 1.0 - ctx.token_price };
    let edge = est_prob - implied_p;
    let min_edge = 0.0;
    ctx.set("debug_est_prob", est_prob);
    ctx.set("debug_implied_p", implied_p);
    ctx.set("debug_edge", edge);
    if edge < min_edge { return; }
"#;

/// Insert edge filter block before the final bet placement.
/// Looks for `if score >= min_score` or `if score >= 3` as insertion marker.
fn inject_edge_filter(content: &str) -> Result<String, String> {
    // Try to find the final bet placement line
    let markers = [
        "if score >= min_score",
        "if score >= 3",
        "ctx.buy(",
        "ctx.sell(",
    ];

    for marker in &markers {
        if let Some(pos) = content.find(marker) {
            // Find the start of the line containing the marker
            let line_start = content[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let before = &content[..line_start];
            let after = &content[line_start..];
            return Ok(format!("{}{}{}", before, EDGE_FILTER_BLOCK, after));
        }
    }

    Err("Could not find bet placement marker (if score >= min_score / ctx.buy / ctx.sell). Add edge filter manually.".to_string())
}
