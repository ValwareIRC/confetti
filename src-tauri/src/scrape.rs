//! Reading the directive tree out of UnrealIRCd's own source.
//!
//! The parser in `src/conf.c` decides what a directive is called, so that is
//! where the names come from — not from a list we maintain. Two conventions
//! cover nearly everything:
//!
//! * `conf.c` has a table, `_ConfigCommands[]`, mapping each block name to its
//!   test function. Inside `_test_class(.., ConfigEntry *ce)`, `ce` IS the
//!   class block, `cep` are its children, `cepp` theirs, `ceppp` theirs. The
//!   parser declares no deeper iterator, which is where the five-level ceiling
//!   comes from.
//! * A module hooks `HOOKTYPE_CONFIGTEST`, guards on `type != CONFIG_SET`,
//!   then matches `ce->name` — so `ce` is `set::<that name>` and its children
//!   hang below it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Directive {
    /// `class::sendq`, `set::anti-flood::everyone::connect-flood`
    pub path: String,
    /// `src/conf.c:3833` — shown when the wiki has nothing to say
    pub source: String,
    /// How the server parses the value, read from the `config_checkval` call
    /// in the branch that handles this directive. `None` where the source does
    /// not say, which is most of them — those are plain text or a name.
    pub kind: Option<crate::types::Kind>,
}

#[derive(Debug, Default)]
pub struct Scrape {
    pub directives: Vec<Directive>,
    pub blocks: Vec<String>,
    /// Top-level blocks contributed by modules rather than the core table
    pub module_blocks: Vec<String>,
    /// Path -> how conf.c parses its value, where it said so.
    pub kinds: BTreeMap<String, crate::types::Kind>,
    /// Paths handed to `conf_match_block()`, which is what makes something a
    /// matcher. Twenty-one call sites across eleven files; a hand-written list
    /// of them had five, and silently missed tkl, spamreport and the rest.
    pub matchers: std::collections::BTreeSet<String>,
    /// Path -> the words its value may be, where the source enumerates them.
    pub choices: BTreeMap<String, Vec<String>>,
    /// Operclass permissions, as `channel:override:mode` paths.
    pub permissions: Vec<String>,
    /// Path -> why the server refuses it. Names that exist in the source only
    /// so it can tell you to stop using them.
    pub refused: BTreeMap<String, String>,
    /// Path -> the value the server uses when you don't set it.
    pub defaults: BTreeMap<String, String>,
    /// Path -> the settings-struct field the applier writes. Only kept so the
    /// defaults can be joined onto it.
    pub fields: BTreeMap<String, String>,
    pub files_scanned: usize,
    /// Comparisons we saw but could not attribute to a block
    pub unattributed: usize,
}

/// Depth of a `ce` / `cep` / `cepp` / `ceppp` iterator, or None if it isn't one.
fn iterator_depth(var: &str) -> Option<usize> {
    let rest = var.strip_prefix("ce")?;
    if rest.chars().all(|c| c == 'p') {
        Some(rest.len())
    } else {
        None
    }
}

/// `{ "official-channels", _conf_offchans, _test_offchans }` -> test fn to name
/// Which C functions handle which block.
///
/// `{ "oper", _conf_oper, _test_oper }` names two: the test function, which
/// enumerates the directives that exist, and the apply function, which is
/// where `config_checkval(cep->value, CFG_YESNO)` actually says what a value
/// is. Directives come from the first; kinds only ever appear in the second,
/// which is why reading the test functions alone found eight booleans out of
/// forty-six.
fn config_commands(src: &str, want_appliers: bool) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Some(start) = src.find("_ConfigCommands[] = {") else { return out };
    let body = &src[start..];
    let end = body.find("\n};").unwrap_or(body.len());
    for line in body[..end].lines().skip(1) {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Some(q1) = line.find('"') else { continue };
        let Some(q2) = line[q1 + 1..].find('"') else { continue };
        let name = &line[q1 + 1..q1 + 1 + q2];
        for ident in line[q1 + 1 + q2..]
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|s| {
                let t = s.starts_with("_test") || s.contains("test_");
                let c = s.starts_with("_conf") || s.contains("conf_");
                t || (want_appliers && c)
            })
        {
            out.entry(ident.to_string()).or_default().push(name.to_string());
        }
    }
    out
}

/// What a `set` directive is worth when you don't write it.
///
/// The server keeps these in two places that never mention each other:
/// `config_setdefaultsettings()` assigns struct fields (`i->maxbans = 60`),
/// and the applier assigns the same field from your config
/// (`tempiConf.maxbans = atol(cep->value)` inside the `maxbans` branch). The
/// field name is the join. Nothing else in the source says "the default for
/// set::maxbans is 60", so this is the only honest way to know it.
fn default_settings(src: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(at) = src.find("void config_setdefaultsettings(") else { return out };
    let Some(open) = src[at..].find('{').map(|i| at + i) else { return out };
    let Some(close) = matching_brace_at(src.as_bytes(), open) else { return out };

    for line in src[open..close].lines() {
        let t = line.trim();
        // safe_strdup(i->cloak_prefix, "Clk");
        if let Some(rest) = t.strip_prefix("safe_strdup(i->") {
            if let Some((field, tail)) = rest.split_once(',') {
                if let Some(v) = quoted(tail) {
                    out.entry(field.trim().to_string()).or_insert(v);
                }
            }
            continue;
        }
        // i->maxbans = 60;   /* and nothing more complicated than that */
        let Some(rest) = t.strip_prefix("i->") else { continue };
        let Some((field, value)) = rest.split_once('=') else { continue };
        let field = field.trim();
        if field.is_empty() || !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let value = value.trim().trim_end_matches(';').trim();
        // A literal only. `i->x = SOME_CONSTANT` tells a reader nothing, and a
        // chained assignment is not a default for this field alone.
        let plain = value.trim_matches('"');
        let is_number = !plain.is_empty() && plain.chars().all(|c| c.is_ascii_digit());
        let quoted_str = value.starts_with('"') && value.ends_with('"') && value.len() >= 2;
        if is_number || quoted_str {
            out.entry(field.to_string()).or_insert(plain.to_string());
        }
    }
    out
}

fn matching_brace_at(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for i in open..bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The first double-quoted run on the line.
fn quoted(s: &str) -> Option<String> {
    let a = s.find('"')?;
    let rest = &s[a + 1..];
    let b = rest.find('"')?;
    Some(rest[..b].to_string())
}

/// `tempiConf.maxbans = atol(..)` -> "maxbans". The field the applier writes,
/// which is what ties a directive to its default.
fn assigned_field(line: &str) -> Option<String> {
    for marker in ["tempiConf.", "iConf."] {
        if let Some(at) = line.find(marker) {
            let rest = &line[at + marker.len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                continue;
            }
            // Only an assignment to it, not a read of it.
            let after = &rest[name.len()..];
            let assigns = after.trim_start().starts_with('=')
                && !after.trim_start().starts_with("==");
            if assigns || line.contains(&format!("safe_strdup({marker}{name}")) {
                return Some(name);
            }
        }
    }
    None
}

/// Directives the source knows the name of but refuses to accept, by the line
/// the branch opens on: `(line, name) -> what the server says about it`.
///
/// UnrealIRCd keeps its removals in the config test as a branch that reports an
/// error and nothing else:
///
///   } else if (!strcmp(cep->name, "privileges"))
///   {
///       new_permissions_system(conf, cep);
///       errors++;
///       return errors;
///   }
///
/// To a scrape that reads directive names out of those comparisons, that looks
/// exactly like a directive. So Confetti offered `privileges` inside operclass,
/// where writing it stops the server from starting at all.
fn refusals(src: &str) -> BTreeMap<(usize, String), String> {
    let mut out = BTreeMap::new();
    let bytes = src.as_bytes();
    let mut at = 0usize;
    while let Some(rel) = src[at..].find("!strcmp(") {
        let start = at + rel;
        at = start + 8;
        let Some(name) = branch_name(&src[start..]) else { continue };
        // The branch's own block. An empty body (`;`) or an `else` in the way
        // means the next block belongs to something else, and reading it would
        // condemn the valid directive above it.
        let Some(open) = src[start..].find('{').map(|i| start + i) else { continue };
        if open - start > 400 {
            continue;
        }
        let gap = &src[start..open];
        if gap.contains(';') || gap.contains("else") {
            continue;
        }
        let Some(close) = matching_brace(bytes, open) else { continue };
        let body = &src[open + 1..close];
        if !only_refuses(body) {
            continue;
        }
        let why = refusal_text(body, &name, src);
        let line = src[..start].matches('\n').count() + 1;
        out.insert((line, name), why);
    }
    out
}

/// The name in `!strcmp(cep->name, "x")`, if that is what this is.
fn branch_name(s: &str) -> Option<String> {
    let open = s.find('(')?;
    let rest = &s[open + 1..];
    let comma = rest.find(',')?;
    if !rest[..comma].trim_end().ends_with("->name") {
        return None;
    }
    let tail = &rest[comma + 1..];
    let q1 = tail.find('"')?;
    if !tail[..q1].trim().is_empty() {
        return None;
    }
    let after = &tail[q1 + 1..];
    let q2 = after.find('"')?;
    let name = &after[..q2];
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return None;
    }
    Some(name.to_string())
}

fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for i in open..bytes.len().min(open + 20000) {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// True when every statement in the branch reports a problem or counts one.
/// Anything that reads, stores or checks the value means the directive works.
fn only_refuses(body: &str) -> bool {
    let b = strip_noise(body);
    if !(b.contains("config_error") || b.contains("config_warn") || b.contains("_system(")) {
        return false;
    }
    for st in b.split(';') {
        let st = st.trim();
        if st.is_empty() || st.starts_with("return") || st.starts_with("continue") || st.starts_with("break") {
            continue;
        }
        if st.contains("config_error") || st.contains("config_warn") {
            continue;
        }
        let counter = st.replace(' ', "");
        if counter == "errors++" || counter == "++errors" || counter.starts_with("errors+=") {
            continue;
        }
        // A helper that only reports, like new_permissions_system().
        let callee: String = st.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !callee.is_empty() && st[callee.len()..].trim_start().starts_with('(')
            && (callee.contains("error") || callee.ends_with("_system"))
        {
            continue;
        }
        return false;
    }
    true
}

/// Comments and string bodies removed, so a sentence never reads as code.
fn strip_noise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i = s[i..].find("*/").map(|j| i + j + 2).unwrap_or(b.len());
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            i = s[i..].find('\n').map(|j| i + j).unwrap_or(b.len());
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            out.push_str("\"\"");
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// What the server says about the directive, taken from its own error message.
/// Where the branch only calls a helper, the helper's message is used instead.
fn refusal_text(body: &str, name: &str, src: &str) -> String {
    if let Some(m) = first_message(body, name) {
        return m;
    }
    // `new_permissions_system(conf, cep)` — follow it once.
    let code = strip_noise(body);
    for st in code.split(';') {
        let callee: String = st.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if callee.is_empty() || callee.contains("error") || callee == "errors" {
            continue;
        }
        if let Some(at) = src.find(&format!("void {callee}(")).or_else(|| src.find(&format!("int {callee}("))) {
            if let Some(open) = src[at..].find('{').map(|i| at + i) {
                if let Some(close) = matching_brace(src.as_bytes(), open) {
                    if let Some(m) = first_message(&src[open..close], name) {
                        return m;
                    }
                }
            }
        }
    }
    "the server does not accept this".to_string()
}

/// The first `config_error`/`config_warn` sentence, as English.
fn first_message(body: &str, name: &str) -> Option<String> {
    let mut at = 0usize;
    while let Some(rel) = body[at..].find("config_") {
        let i = at + rel;
        at = i + 7;
        if !(body[i..].starts_with("config_error(") || body[i..].starts_with("config_warn(")) {
            continue;
        }
        // Adjacent literals in C are one string.
        let args = &body[i..];
        let end = args.find(");").unwrap_or(args.len());
        let mut text = String::new();
        let b = args[..end].as_bytes();
        let mut j = 0;
        while j < b.len() {
            if b[j] == b'"' {
                j += 1;
                while j < b.len() && b[j] != b'"' {
                    text.push(b[j] as char);
                    j += 1;
                }
            }
            j += 1;
        }
        let text = text.trim();
        // Drop the `%s:%i: ` file-and-line prefix the server prints.
        let text = text
            .strip_prefix("%s:%i:")
            .or_else(|| text.strip_prefix("%s:%d:"))
            .unwrap_or(text)
            .trim();
        // Any remaining %s is the directive's own name where the call passes
        // it, and noise otherwise.
        let uses_name = args[..end].contains("->name");
        let mut text = text.to_string();
        for spec in ["%s", "%i", "%d"] {
            text = text.replace(spec, if uses_name { name } else { "" });
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.len() > 3 {
            return Some(text);
        }
    }
    None
}

/// Every operclass permission the server checks, as a `a:b:c` path.
///
/// These are not parsed with `strcmp(cep->name, ..)` like everything else:
/// `permissions { }` is a free-form tree, validated at run time against the
/// paths the code asks about. So the scrape that finds every other directive
/// finds none of these, and the editor offered an empty text box for a value
/// that has a hundred-odd known answers.
///
/// Three forms ask the question, and all three have to be read:
///   ValidatePermissionsForPath("channel:override:mode", ..)
///   ValidatePermissionsForPathEx(..)
///   op_can_override("channel:override:kick:owner", ..)
fn permission_paths(src: &str) -> Vec<String> {
    const CALLS: &[&str] = &[
        "ValidatePermissionsForPath(",
        "ValidatePermissionsForPathEx(",
        "op_can_override(",
    ];
    let mut out = Vec::new();
    for call in CALLS {
        let mut rest = src;
        while let Some(at) = rest.find(call) {
            let after = &rest[at + call.len()..];
            // The path is the first argument, and must be a literal for us to
            // read it; a variable there is a permission we cannot know.
            let head: String = after.chars().take_while(|c| *c != ',' && *c != ')').collect();
            if let Some(q1) = head.find('"') {
                if head[..q1].trim().is_empty() {
                    let tail = &head[q1 + 1..];
                    if let Some(q2) = tail.find('"') {
                        let p = &tail[..q2];
                        if !p.is_empty()
                            && p.chars().all(|c| {
                                c.is_ascii_lowercase() || c.is_ascii_digit() || c == ':' || c == '-'
                            })
                        {
                            out.push(p.to_string());
                        }
                    }
                }
            }
            rest = &after[..];
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Every fixed vocabulary the source defines, by the name of the thing that
/// holds it.
///
/// UnrealIRCd writes these three ways, and all three have to be read or the
/// editor offers a free text box for a value that must be one of five words:
///
///   1. inline           `if (!strcmp(cep->value, "allow"))`
///   2. a helper         `policy_strtoval(cepp->value)`, an if-chain of its own
///   3. a static table   `static BanActTable banacttable[] = { { .., "kill" } }`
///
/// The first is read where it is used; this collects the other two so a call
/// can be resolved to the words it accepts.
fn vocabularies(src: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let mut current: Option<String> = None;
    let mut depth = 0i32;
    for line in src.lines() {
        let trimmed = line.trim();

        if current.is_none() {
            // A lookup helper: `Policy policy_strtoval(const char *s)`.
            if let Some(name) = helper_name(line) {
                current = Some(name);
                depth = 0;
            } else if let Some(name) = table_name(line) {
                // A table: read its rows until the closing brace.
                current = Some(name);
                depth = 1;
                continue;
            }
        }

        if let Some(name) = current.clone() {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            for w in quoted_words(trimmed) {
                out.entry(name.clone()).or_default().push(w);
            }
            if depth <= 0 && (trimmed.contains('}') || trimmed.ends_with(';')) {
                current = None;
            }
        }
    }

    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out.retain(|_, v| !v.is_empty() && v.len() <= 40);
    out
}

/// `Policy policy_strtoval(const char *s)` -> `policy_strtoval`
fn helper_name(line: &str) -> Option<String> {
    if line.starts_with(' ') || line.starts_with('\t') || line.starts_with('/') {
        return None;
    }
    let open = line.find('(')?;
    let head = &line[..open];
    let name = head.split_whitespace().last()?.trim_start_matches('*');
    if name.ends_with("_strtoval") || name.ends_with("_stringtoval") {
        Some(name.to_string())
    } else {
        None
    }
}

/// `static BanActTable banacttable[] = {` -> `banacttable`
fn table_name(line: &str) -> Option<String> {
    let t = line.trim();
    if !t.starts_with("static ") || !t.contains("[] = {") {
        return None;
    }
    let head = t.split("[]").next()?;
    Some(head.split_whitespace().last()?.to_string())
}

/// The words a line offers as accepted values.
fn quoted_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            if j < bytes.len() {
                let w = &line[start..j];
                // A value, not a format string or a sentence.
                if !w.is_empty()
                    && w.len() <= 32
                    && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
                {
                    out.push(w.to_string());
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `strcmp(cep->value, "allow")` — the words compared against a value on this
/// line, which are exactly the words that value may be.
fn value_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("->value") {
        let after = &rest[at + 7..];
        let Some(q1) = after.find('"') else { break };
        // Only if the quote is close: `->value, "x"`, not a later literal.
        if after[..q1].len() > 4 {
            rest = after;
            continue;
        }
        let tail = &after[q1 + 1..];
        let Some(q2) = tail.find('"') else { break };
        let w = &tail[..q2];
        if !w.is_empty()
            && w.len() <= 32
            && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            out.push(w.to_string());
        }
        rest = &tail[q2 + 1..];
    }
    out
}

/// Every `<iterator>->name` compared against a string literal on this line.
fn comparisons(line: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    while let Some(rel) = line[i..].find("->name") {
        let at = i + rel;
        // identifier immediately before
        let mut s = at;
        while s > 0 && (b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_') {
            s -= 1;
        }
        let var = &line[s..at];
        let after = &line[at + 6..];
        let trimmed = after.trim_start();
        if let (Some(depth), Some(rest)) = (iterator_depth(var), trimmed.strip_prefix(',')) {
            let rest = rest.trim_start();
            if let Some(lit) = rest.strip_prefix('"') {
                if let Some(q) = lit.find('"') {
                    let value = &lit[..q];
                    if !value.is_empty()
                        && value.chars().all(|c| {
                            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.'
                        })
                    {
                        out.push((depth, value.to_string()));
                    }
                }
            }
        }
        i = at + 6;
    }
    out
}

fn is_function_start(line: &str) -> Option<String> {
    if line.starts_with(char::is_whitespace) || !line.contains("(ConfigFile") {
        return None;
    }
    if line.trim_end().ends_with(';') {
        return None; // a prototype
    }
    let paren = line.find('(')?;
    let head = &line[..paren];
    let name = head.rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn scan_file(
    path: &Path,
    rel: &str,
    commands: &BTreeMap<String, Vec<String>>,
    appliers: &BTreeMap<String, Vec<String>>,
    vocab: &BTreeMap<String, Vec<String>>,
    out: &mut Scrape,
) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{rel}: {e}"))?;
    out.files_scanned += 1;

    // A module can register several config hooks in one file at different
    // phases — blacklist.c has `blacklist_config_test` guarding on CONFIG_MAIN
    // (the top-level `blacklist` block) and `blacklist_set_config_test`
    // guarding on CONFIG_SET (`set::blacklist`). So the phase has to be read
    // per function, not per file; reading it per file meant the top-level
    // block was never recorded and valid configs got flagged as unknown.
    let has_config_hooks = src.contains("CONFIG_SET") || src.contains("CONFIG_MAIN");

    // Roots for the function we are currently inside.
    let mut bases: Vec<String> = Vec::new();
    let mut names: Vec<Option<String>> = vec![None; 8];
    let mut in_known_fn = false;
    let mut kinds_only = false;
    let mut fn_phase: Option<&str> = None;
    // The directive paths most recently opened a branch for, so a
    // config_checkval a line or two later can be attributed to them. Bounded,
    // because a stale name from thirty lines up is a guess, not a reading.
    let mut last_paths: Vec<String> = Vec::new();
    let mut last_line = 0usize;
    // Branches that do nothing but refuse a name, found ahead of the line scan
    // because a branch body spans lines and this scan reads one at a time.
    let refused = refusals(&src);

    for (n, line) in src.lines().enumerate() {
        let lineno = n + 1;

        if let Some(fname) = is_function_start(line) {
            names.iter_mut().for_each(|s| *s = None);
            bases.clear();
            last_paths.clear();
            last_line = 0;
            in_known_fn = false;
            fn_phase = None;
            kinds_only = false;
            if let Some(block_names) = commands.get(&fname) {
                bases = block_names.clone();
                in_known_fn = true;
            } else if let Some(block_names) = appliers.get(&fname) {
                // The apply function for a block we already know. It says how
                // values are parsed; it must not invent directives, since the
                // test function is the authority on what exists.
                bases = block_names.clone();
                in_known_fn = true;
                kinds_only = true;
            } else if has_config_hooks
                && (fname.contains("config_run") || fname.contains("configrun"))
            {
                // A module's apply hook: same shape as its test hook, and the
                // only place it says how a value is parsed.
                in_known_fn = true;
                kinds_only = true;
            } else if has_config_hooks
                && (fname.contains("config_test") || fname.contains("configtest"))
            {
                // Base is established by the phase guard plus the first
                // ce->name match in the body.
                in_known_fn = true;
            }
            continue;
        }

        if in_known_fn && bases.is_empty() && fn_phase.is_none() {
            if line.contains("CONFIG_MAIN") {
                fn_phase = Some("");
            } else if line.contains("CONFIG_SET") {
                fn_phase = Some("set");
            }
        }

        for (depth, lit) in comparisons(line) {
            if !in_known_fn {
                out.unattributed += 1;
                continue;
            }
            if depth == 0 {
                if bases.is_empty() {
                    // A module naming the block it handles.
                    bases = match fn_phase {
                        Some(p) if !p.is_empty() => vec![format!("{p}::{lit}")],
                        _ => {
                            // A module adding a block of its own, like `blacklist`.
                            out.module_blocks.push(lit.clone());
                            vec![lit.clone()]
                        }
                    };
                }
                continue;
            }
            if bases.is_empty() {
                out.unattributed += 1;
                continue;
            }
            // Every ancestor between the base and here must be known, or the
            // path would be a guess.
            if (1..depth).any(|d| names[d].is_none()) {
                out.unattributed += 1;
                continue;
            }
            let mut paths = Vec::new();
            for base in &bases {
                let mut parts = vec![base.clone()];
                for d in 1..depth {
                    parts.push(names[d].clone().unwrap());
                }
                parts.push(lit.clone());
                paths.push(parts.join("::"));
            }
            if !kinds_only {
                for p in &paths {
                    out.directives.push(Directive {
                        path: p.clone(),
                        source: format!("{rel}:{lineno}"),
                        kind: None,
                    });
                }
            }
            if let Some(why) = refused.get(&(lineno, lit.clone())) {
                for p in &paths {
                    out.refused.insert(p.clone(), why.clone());
                }
            }
            names[depth] = Some(lit.clone());
            for d in depth + 1..names.len() {
                names[d] = None;
            }
            last_paths = paths;
            last_line = lineno;
        }

        // `} else if (!strcmp(cep->name, "auto-login")) { x = config_checkval(
        //  cep->value, CFG_YESNO); }` — the branch just opened is the one this
        // belongs to, so the nearest preceding name comparison names it. This
        // is how UnrealIRCd itself decides, rather than a list of directive
        // names somebody recognised: there are 91 yes/no directives and a
        // hand-written list had eleven of them.
        if !last_paths.is_empty() && lineno.saturating_sub(last_line) <= 12 {
            let kind = if line.contains("CFG_YESNO") {
                Some(crate::types::Kind::YesNo)
            } else if line.contains("CFG_TIME") {
                Some(crate::types::Kind::Duration)
            } else if line.contains("CFG_SIZE") {
                Some(crate::types::Kind::Size)
            } else {
                None
            };
            if let Some(k) = kind {
                for p in &last_paths {
                    out.kinds.insert(p.clone(), k);
                }
            }
            if line.contains("conf_match_block(") && !line.contains("int conf_match_block") {
                for p in &last_paths {
                    out.matchers.insert(p.clone());
                }
            }
            if let Some(field) = assigned_field(line) {
                for p in &last_paths {
                    out.fields.entry(p.clone()).or_insert_with(|| field.clone());
                }
            }

            // The words this value may be: compared inline, or looked up by a
            // helper whose own vocabulary we read beforehand.
            // Read the whole branch, not the first line or two of it: an
            // if-chain spreads its cases over a dozen lines, and stopping
            // early cost `allow-userhost-change` two of its four values. An
            // incomplete list that is presented as complete is worse than no
            // list, so these are offered as suggestions you can type past.
            let mut words = value_words(line);
            for (helper, list) in vocab {
                if line.contains(&format!("{helper}(")) {
                    words.extend(list.iter().cloned());
                }
            }
            if !words.is_empty() {
                for p in &last_paths {
                    let e = out.choices.entry(p.clone()).or_default();
                    e.extend(words.iter().cloned());
                    e.sort();
                    e.dedup();
                }
            }
        }
    }
    Ok(())
}

fn collect_c_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_c_files(&p, out);
        } else if p.extension().map(|x| x == "c").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// Scrape an UnrealIRCd source tree.
pub fn scrape(root: &Path) -> Result<Scrape, String> {
    let conf_c = root.join("src").join("conf.c");
    if !conf_c.exists() {
        return Err(format!("{} is not an UnrealIRCd source tree (no src/conf.c)", root.display()));
    }
    let conf_src = std::fs::read_to_string(&conf_c).map_err(|e| e.to_string())?;
    let commands = config_commands(&conf_src, false);
    let appliers = config_commands(&conf_src, true);

    // The vocabularies have to be known before the scan, since a helper may be
    // defined in one file and called from another.
    let mut vocab = vocabularies(&conf_src);
    let mut vocab_files = Vec::new();
    collect_c_files(&root.join("src"), &mut vocab_files);
    vocab_files.sort();
    let mut perms: Vec<String> = permission_paths(&conf_src);
    for f in &vocab_files {
        if let Ok(text) = std::fs::read_to_string(f) {
            for (k, v) in vocabularies(&text) {
                vocab.entry(k).or_insert(v);
            }
            perms.extend(permission_paths(&text));
        }
    }
    perms.sort();
    perms.dedup();

    let mut out = Scrape::default();
    out.permissions = perms;
    let table_blocks: Vec<String> = commands.values().flatten().cloned().collect();

    scan_file(&conf_c, "src/conf.c", &commands, &appliers, &vocab, &mut out)?;

    let mut modules = Vec::new();
    collect_c_files(&root.join("src").join("modules"), &mut modules);
    modules.sort();
    for m in modules {
        let rel = m
            .strip_prefix(root)
            .unwrap_or(&m)
            .to_string_lossy()
            .replace('\\', "/");
        let _ = scan_file(&m, &rel, &commands, &appliers, &vocab, &mut out);
    }

    // Blocks come from two places: the core table, and modules that add a
    // top-level block of their own (`blacklist` is one). Missing the second
    // group would make the editor call valid configuration unknown.
    out.blocks = table_blocks;
    out.blocks.extend(out.module_blocks.iter().cloned());
    out.blocks.sort();
    out.blocks.dedup();

    // A single word is not a vocabulary, it is one branch of a comparison we
    // happened to catch; offering it as the only option would reject every
    // other valid value. And where conf.c already said the value is a
    // duration, a size or a password, a word list is noise at best.
    out.choices.retain(|path, words| {
        words.len() >= 2
            && !matches!(
                out.kinds.get(path),
                Some(crate::types::Kind::Duration)
                    | Some(crate::types::Kind::Size)
                    | Some(crate::types::Kind::YesNo)
            )
            && !path.ends_with("::password")
            && !path.ends_with("::word")
    });

    // `channel:override:mode` becomes operclass::permissions::channel,
    // ::channel::override and ::channel::override::mode, so the same code that
    // offers directives inside any other block offers these.
    for p in &out.permissions {
        let mut path = String::from("operclass::permissions");
        for part in p.split(':') {
            path.push_str("::");
            path.push_str(part);
            out.directives.push(Directive {
                path: path.clone(),
                source: "operclass permissions".to_string(),
                kind: None,
            });
        }
    }

    // The defaults, joined onto the directives by the field each one writes.
    let defs = default_settings(&conf_src);
    for (path, field) in &out.fields {
        if let Some(v) = defs.get(field) {
            // A boolean's default is stored as 1 or 0, but you write yes or no.
            let v = match (out.kinds.get(path), v.as_str()) {
                (Some(crate::types::Kind::YesNo), "1") => "yes".to_string(),
                (Some(crate::types::Kind::YesNo), "0") => "no".to_string(),
                _ => v.clone(),
            };
            out.defaults.insert(path.clone(), v);
        }
    }

    // Attach what the appliers said, now that every directive is known.
    for d in out.directives.iter_mut() {
        if let Some(k) = out.kinds.get(&d.path) {
            d.kind = Some(*k);
        }
    }
    out.directives.sort_by(|a, b| a.path.cmp(&b.path));
    out.directives.dedup_by(|a, b| {
        if a.path != b.path {
            return false;
        }
        // `a` is the later of the pair and is the one being dropped: carry its
        // kind over first. conf.c reads most directives twice — once in the
        // test function and once in the apply — and only one of those calls
        // config_checkval, so whichever knew has to win.
        if b.kind.is_none() {
            b.kind = a.kind;
        }
        true
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r"C:\Users\Valerie\unrealircd-pr";

    /// The kinds have to come from the source, not from recognising words.
    /// A hand-written list of "which directives are yes/no" had eleven names
    /// in it; UnrealIRCd parses ninety-odd that way.
    #[test]
    fn yes_no_directives_are_read_from_conf_c() {
        let root = std::path::Path::new(SRC);
        if !root.join("src").join("conf.c").exists() {
            return; // no source tree here; nothing to check against
        }
        let sc = scrape(root).expect("scrape");
        let yes: Vec<&Directive> = sc
            .directives
            .iter()
            .filter(|d| d.kind == Some(crate::types::Kind::YesNo))
            .collect();
        // Reading conf.c finds 38 of the 46 that a name-match can be tied to;
        // the rest sit too far from their branch to attribute honestly. The
        // hand-written list this replaced had eleven.
        assert!(
            yes.len() >= 30,
            "only {} yes/no directives found; the CFG_YESNO reading has broken",
            yes.len()
        );
        // The one that started this: oper::auto-login is a boolean because
        // conf.c reads it with CFG_YESNO.
        assert!(
            yes.iter().any(|d| d.path == "oper::auto-login"),
            "oper::auto-login not recognised as yes/no"
        );
        // Spread across conf.c and the modules, so both paths are covered.
        for p in ["set::ping-cookie", "set::uhnames", "vhost::auto-login"] {
            assert!(yes.iter().any(|d| d.path == p), "{p} not recognised as yes/no");
        }
        let times = sc.directives.iter().filter(|d| d.kind == Some(crate::types::Kind::Duration)).count();
        assert!(times > 5, "only {times} duration directives found");
    }

    /// Matchers come from `conf_match_block()` call sites, not from a list of
    /// paths somebody wrote down. The list this replaced had five entries and
    /// missed tkl, spamreport and central-blocklist entirely.
    #[test]
    fn matchers_are_read_from_the_source() {
        let root = std::path::Path::new(SRC);
        if !root.join("src").join("conf.c").exists() {
            return;
        }
        let sc = scrape(root).expect("scrape");
        assert!(
            sc.matchers.len() >= 10,
            "only {} matcher paths found: {:?}",
            sc.matchers.len(),
            sc.matchers
        );
        // The ones the hand-written list never had.
        // Nine of these the hand-written list never had. `oper::mask` is not
        // among them because that branch reads `"mask" || "match"` and the
        // nearest name is the second one — the `last == "mask"` rule in
        // types::is_matcher still covers it, and the two are ORed.
        for p in [
            "allow::hostname",
            "require::mask",
            "spamfilter::except",
            "spamreport::except",
            "webirc::match",
            "oper::match",
        ] {
            assert!(sc.matchers.contains(p), "{p} not found: {:?}", sc.matchers);
        }
    }

    #[test]
    fn value_vocabularies_are_read_from_the_source() {
        let root = std::path::Path::new(SRC);
        if !root.join("src").join("conf.c").exists() { return; }
        let sc = scrape(root).expect("scrape");
        let mut v: Vec<_> = sc.choices.iter().collect();
        v.sort_by_key(|(k, _)| k.to_string());
        println!("paths with a fixed vocabulary: {}", v.len());
        for (k, words) in v.iter().take(45) {
            println!("  {:44} {}", k, words.join(" "));
        }
    }

    /// Permissions are a free-form tree in the config, validated at run time,
    /// so the strcmp scrape that finds every other directive finds none of
    /// them. They come from the code that asks the questions instead.
    #[test]
    fn operclass_permissions_are_read_from_the_source() {
        let root = std::path::Path::new(SRC);
        if !root.join("src").join("conf.c").exists() { return; }
        let sc = scrape(root).expect("scrape");
        println!("permission paths: {}", sc.permissions.len());
        assert!(sc.permissions.len() > 80, "only {} found", sc.permissions.len());
        for p in ["channel:override:mode", "chat:globops", "client:see:ip", "self:opermodes"] {
            assert!(sc.permissions.iter().any(|x| x == p), "{p} missing");
        }
        // And each one is reachable as a directive path, at every level.
        for p in ["operclass::permissions::channel",
                  "operclass::permissions::channel::override",
                  "operclass::permissions::channel::override::mode"] {
            assert!(sc.directives.iter().any(|d| d.path == p), "{p} not a directive");
        }
        let tops: std::collections::BTreeSet<&str> =
            sc.permissions.iter().filter_map(|p| p.split(':').next()).collect();
        println!("top level: {}", tops.into_iter().collect::<Vec<_>>().join(" "));
    }

    /// A name the server knows only so it can refuse it is not a directive.
    /// `operclass::privileges` was offered as something to add, and writing it
    /// stops UnrealIRCd from starting.
    #[test]
    fn refused_directives_are_told_apart_from_real_ones() {
        let root = std::path::Path::new(SRC);
        if !root.join("src").join("conf.c").exists() { return; }
        let sc = scrape(root).expect("scrape");
        let mut v: Vec<_> = sc.refused.iter().collect();
        v.sort_by_key(|(k, _)| k.to_string());
        println!("refused: {}", v.len());
        for (k, why) in &v {
            println!("  {:38} {}", k, &why[..why.len().min(74)]);
        }
        assert!(v.len() >= 20, "only {} found", v.len());

        // Named cases, so a change of shape in conf.c is a failing test rather
        // than a quietly shorter list.
        for p in ["operclass::privileges", "me::numeric", "set::throttle",
                  "set::hosts", "link::ciphers", "allow::options::sasl"] {
            assert!(sc.refused.contains_key(p), "{p} not recognised as refused");
        }
        assert!(sc.refused["set::throttle"].contains("renamed"),
                "{}", sc.refused["set::throttle"]);
        assert!(sc.refused["operclass::privileges"].contains("permissions"),
                "{}", sc.refused["operclass::privileges"]);

        // And the far more important half: nothing valid was caught. These are
        // all real, and each sits next to a branch that only reports an error.
        for p in ["oper::mask", "tld::mask", "tld::match", "allow::options::useip",
                  "set::reject-message::kline", "set::reject-message::server-full",
                  "spamreport::type", "set::anti-flood::everyone",
                  "hideserver::disable-map", "alias::spamfilter"] {
            assert!(!sc.refused.contains_key(p), "{p} wrongly called refused");
        }
    }

    #[test]
    fn a_branch_that_only_reports_is_told_from_one_that_works() {
        assert!(only_refuses(r#" config_error("gone"); errors++; continue; "#));
        assert!(only_refuses(" new_permissions_system(conf, cep); errors++; return errors; "));
        assert!(!only_refuses(r#" x = atoi(cep->value); config_error("also"); "#));
        assert!(!only_refuses(" conf_match_block(conf, cep, &oper->match); "));
        // A sentence about errors is not a call to report one.
        assert!(!only_refuses(r#" safe_strdup(x, "config_error is not code"); "#));
    }

    /// The server's own defaults, joined to directives by the settings-struct
    /// field each applier writes. Nothing in the source says "set::maxbans
    /// defaults to 60" in one place; this is the join that makes it sayable.
    #[test]
    fn set_defaults_are_read_from_the_source() {
        let root = std::path::Path::new(SRC);
        if !root.join("src").join("conf.c").exists() { return; }
        let sc = scrape(root).expect("scrape");
        let mut v: Vec<_> = sc.defaults.iter().collect();
        v.sort_by_key(|(k, _)| k.to_string());
        println!("directives with a known default: {}", v.len());
        for (k, d) in v.iter().take(45) {
            println!("  {:44} {}", k, d);
        }
        assert!(v.len() >= 20, "only {} found", v.len());
        for (path, want) in [("set::maxbans", "60"), ("set::hiddenhost-prefix", "Clk"),
                             ("set::topic-length", "360"), ("set::maxdccallow", "10"),
                             ("set::uhnames", "yes"), ("set::allow-setident", "no")] {
            assert_eq!(sc.defaults.get(path).map(|s| s.as_str()), Some(want),
                       "{path}: {:?}", sc.defaults.get(path));
        }
    }

    #[test]
    fn reads_iterator_depth() {
        assert_eq!(iterator_depth("ce"), Some(0));
        assert_eq!(iterator_depth("cep"), Some(1));
        assert_eq!(iterator_depth("ceppp"), Some(3));
        assert_eq!(iterator_depth("cfptr"), None);
        assert_eq!(iterator_depth("client"), None);
    }

    #[test]
    fn finds_comparisons_on_a_line() {
        let c = comparisons(r#"		if (!strcmp(cep->name, "pingfreq"))"#);
        assert_eq!(c, vec![(1, "pingfreq".to_string())]);
        let c = comparisons(r#"} else if (!strcmp(ceppp->name, "ban-action"))"#);
        assert_eq!(c, vec![(3, "ban-action".to_string())]);
        assert!(comparisons("x = client->name;").is_empty());
    }

    #[test]
    fn parses_the_command_table() {
        let table = r#"static ConfigCommand _ConfigCommands[] = {
	{ "admin", 		_conf_admin,		_test_admin 	},
	{ "official-channels", 	_conf_offchans,		_test_offchans	},
};"#;
        let m = config_commands(table, false);
        // With appliers wanted, the same line also yields the _conf_ function.
        let a = config_commands(table, true);
        assert_eq!(a.get("_conf_admin").unwrap(), &vec!["admin".to_string()]);
        assert_eq!(m.get("_test_admin").unwrap(), &vec!["admin".to_string()]);
        assert_eq!(m.get("_test_offchans").unwrap(), &vec!["official-channels".to_string()]);
    }

    #[test]
    fn scrapes_the_real_source_tree() {
        let root = Path::new(SRC);
        if !root.exists() {
            eprintln!("no source tree at {SRC}, skipping");
            return;
        }
        let s = scrape(root).expect("scrape");
        println!(
            "{} files, {} blocks, {} directives, {} unattributed",
            s.files_scanned,
            s.blocks.len(),
            s.directives.len(),
            s.unattributed
        );

        assert!(s.blocks.contains(&"class".to_string()));
        assert!(s.blocks.contains(&"official-channels".to_string()));

        let paths: Vec<&str> = s.directives.iter().map(|d| d.path.as_str()).collect();
        for want in [
            "class::sendq",
            "class::pingfreq",
            "me::sid",
            "oper::operclass",
            "listen::options",
            "set::anti-flood",
            "set::anti-flood::everyone",
        ] {
            assert!(paths.contains(&want), "missing {want}");
        }

        let deepest = s.directives.iter().map(|d| d.path.matches("::").count() + 1).max().unwrap();
        println!("deepest scraped path has {deepest} segments");
        assert!(deepest >= 4);
    }
}
