//! Structural gate for the composite action and the workflow that dogfoods it.
//!
//! `action.yml` is only ever exercised for real inside GitHub Actions, where a
//! mistake costs a red pull request and a round trip. These tests read it the way
//! the runner does — as structure, not as text — so a renamed input, a step that
//! forgot its shell, or an untrusted event value spliced into a script fails the
//! build instead of a run.
//!
//! The reader below understands the YAML subset these files are written in
//! (block mappings, block sequences, and block scalars). It is deliberately
//! strict: anything it cannot make sense of is a panic naming the line, which is
//! the syntax check a hand-written action.yml otherwise never gets.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// A parsed YAML node.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Scalar(String),
    Map(BTreeMap<String, Node>),
    List(Vec<Node>),
}

impl Node {
    /// The value at `key`, or a panic naming what was actually there.
    fn get(&self, key: &str) -> &Node {
        match self {
            Node::Map(entries) => entries
                .get(key)
                .unwrap_or_else(|| panic!("no `{key}` here; found {:?}", entries.keys())),
            other => panic!("`{key}` was looked up in a {other:?}"),
        }
    }

    /// The value at `key`, if there is one.
    fn find(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries.get(key),
            _ => None,
        }
    }

    /// This node's keys, in order.
    fn keys(&self) -> Vec<&str> {
        match self {
            Node::Map(entries) => entries.keys().map(String::as_str).collect(),
            other => panic!("expected a mapping, got {other:?}"),
        }
    }

    fn scalar(&self) -> &str {
        match self {
            Node::Scalar(value) => value,
            other => panic!("expected a scalar, got {other:?}"),
        }
    }

    fn list(&self) -> &[Node] {
        match self {
            Node::List(items) => items,
            other => panic!("expected a sequence, got {other:?}"),
        }
    }
}

/// A line that carries content, with its indentation.
struct Line<'a> {
    indent: usize,
    text: &'a str,
    number: usize,
}

/// Every line that is neither blank nor a comment.
fn meaningful(text: &str) -> Vec<Line<'_>> {
    text.lines()
        .enumerate()
        .filter_map(|(index, raw)| {
            let trimmed = raw.trim_start();
            (!trimmed.is_empty() && !trimmed.starts_with('#')).then(|| Line {
                indent: raw.len() - trimmed.len(),
                text: trimmed.trim_end(),
                number: index + 1,
            })
        })
        .collect()
}

/// Parse the YAML subset these files use.
fn parse(text: &str) -> Node {
    let lines = meaningful(text);
    let mut cursor = 0;
    let node = parse_block(
        &lines,
        &mut cursor,
        lines.first().map_or(0, |line| line.indent),
    );
    assert_eq!(
        cursor,
        lines.len(),
        "line {} is not part of the document",
        lines[cursor].number
    );
    node
}

/// The block starting at `cursor`, made of every line indented by `indent`.
fn parse_block(lines: &[Line<'_>], cursor: &mut usize, indent: usize) -> Node {
    if lines
        .get(*cursor)
        .is_some_and(|line| line.text.starts_with("- ") || line.text == "-")
    {
        let mut items = Vec::new();
        while lines
            .get(*cursor)
            .is_some_and(|line| line.indent == indent && line.text.starts_with("- "))
        {
            let inline = lines[*cursor].text[2..].trim().to_string();
            *cursor += 1;
            items.push(parse_entries(lines, cursor, indent + 2, Some(inline)));
        }
        return Node::List(items);
    }
    parse_entries(lines, cursor, indent, None)
}

/// A mapping — optionally opened by `inline`, the text that followed a `- `.
fn parse_entries(
    lines: &[Line<'_>],
    cursor: &mut usize,
    indent: usize,
    inline: Option<String>,
) -> Node {
    let mut entries = BTreeMap::new();
    if let Some(first) = inline {
        // A sequence item that is a plain scalar rather than a mapping.
        let Some((key, rest)) = split_entry(&first) else {
            return Node::Scalar(unquote(&first));
        };
        insert(&mut entries, key, rest, lines, cursor, indent);
    }
    while let Some(line) = lines.get(*cursor) {
        if line.indent < indent || line.text.starts_with("- ") {
            break;
        }
        assert_eq!(
            line.indent, indent,
            "line {} is indented {} where {indent} was expected",
            line.number, line.indent
        );
        let text = line.text.to_string();
        let (key, rest) = split_entry(&text)
            .unwrap_or_else(|| panic!("line {} is not a `key: value` entry", line.number));
        *cursor += 1;
        insert(&mut entries, key, rest, lines, cursor, indent);
    }
    Node::Map(entries)
}

/// Record one mapping entry, reading its nested block when it has one.
fn insert(
    entries: &mut BTreeMap<String, Node>,
    key: String,
    rest: String,
    lines: &[Line<'_>],
    cursor: &mut usize,
    indent: usize,
) {
    let value = if rest.is_empty() {
        match lines.get(*cursor) {
            // A sequence may sit at its parent's own indentation.
            Some(line) if line.indent > indent => parse_block(lines, cursor, line.indent),
            Some(line) if line.indent == indent && line.text.starts_with("- ") => {
                parse_block(lines, cursor, indent)
            }
            _ => Node::Scalar(String::new()),
        }
    } else if matches!(rest.as_str(), "|" | ">" | "|-" | ">-" | "|+" | ">+") {
        Node::Scalar(block_scalar(lines, cursor, indent))
    } else {
        Node::Scalar(unquote(&rest))
    };
    assert!(
        entries.insert(key.clone(), value).is_none(),
        "`{key}` is defined twice"
    );
}

/// The indented text under a `|` or `>` marker, joined with spaces.
fn block_scalar(lines: &[Line<'_>], cursor: &mut usize, indent: usize) -> String {
    let mut parts = Vec::new();
    while let Some(line) = lines.get(*cursor) {
        if line.indent <= indent {
            break;
        }
        parts.push(line.text.to_string());
        *cursor += 1;
    }
    parts.join(" ")
}

/// Split `key: value`, leaving a `://` inside a value alone.
fn split_entry(text: &str) -> Option<(String, String)> {
    let colon = text.find(':')?;
    let rest = &text[colon + 1..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((unquote(&text[..colon]), rest.trim().to_string()))
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// Every `run:` step in a composite action or a workflow job.
fn run_steps(steps: &[Node]) -> Vec<&Node> {
    steps
        .iter()
        .filter(|step| step.find("run").is_some())
        .collect()
}

fn action() -> Node {
    parse(&read("action.yml"))
}

#[test]
fn the_action_declares_the_documented_inputs_and_defaults() {
    let action = action();
    let inputs = action.get("inputs");
    assert_eq!(
        inputs.keys(),
        vec!["diff-base", "github-token", "paths", "version"],
        "the action's inputs changed"
    );
    assert_eq!(
        inputs.get("github-token").get("default").scalar(),
        "${{ github.token }}"
    );
    assert_eq!(inputs.get("version").get("default").scalar(), "latest");
    // Both optional: an empty base means "the pull request's base branch" and
    // empty paths mean "the whole repository".
    for input in ["diff-base", "paths"] {
        assert_eq!(inputs.get(input).get("default").scalar(), "", "{input}");
    }
}

#[test]
fn the_action_exposes_the_count_and_the_report_it_produced() {
    let outputs = action().get("outputs").clone();
    assert_eq!(outputs.keys(), vec!["count", "report-path"]);
    assert_eq!(
        outputs.get("count").get("value").scalar(),
        "${{ steps.scan.outputs.count }}"
    );
    assert_eq!(
        outputs.get("report-path").get("value").scalar(),
        "${{ steps.scan.outputs.report-path }}"
    );
}

#[test]
fn every_step_of_the_composite_declares_the_shell_it_runs_in() {
    let action = action();
    assert_eq!(action.get("runs").get("using").scalar(), "composite");
    let steps = action.get("runs").get("steps").list();
    assert!(!steps.is_empty(), "the composite runs no steps");
    for step in run_steps(steps) {
        assert_eq!(
            step.get("shell").scalar(),
            "bash",
            "a composite `run:` step without `shell: bash` never starts"
        );
    }
}

/// Every array expansion, and whether it survives the oldest bash the action
/// runs under.
///
/// A macOS runner's `/bin/bash` is 3.2, where `set -u` rejects `"${a[@]}"` as an
/// unbound variable whenever the array is empty — the state an unset `paths`
/// input leaves the scan step in. Bash 4.4 and later expand it to nothing, so
/// the fault is invisible on Linux and Windows. The `${a[@]+"${a[@]}"}`
/// alternate form means the same thing to every bash, and is what this insists
/// on; the inner half of that form is itself an expansion, so it is recognized
/// rather than reported.
fn unguarded_array_expansions(script: &str) -> Vec<String> {
    let mut unguarded = Vec::new();
    for subscript in ["[@]", "[*]"] {
        for (at, _) in script.match_indices(subscript) {
            let head = &script[..at];
            let Some(open) = head.rfind("${") else {
                continue;
            };
            let name = &head[open + 2..];
            let guard = format!("${{{name}{subscript}+\"");
            let outer = format!("+\"${{{name}{subscript}}}\"}}");
            if head[..open].ends_with(&guard) || script[at + subscript.len()..].starts_with(&outer)
            {
                continue;
            }
            unguarded.push(format!("${{{name}{subscript}}}"));
        }
    }
    unguarded
}

/// The action's own scripts, as the shell receives them.
fn action_scripts() -> Vec<(String, String)> {
    let mut scripts: Vec<(String, String)> = action()
        .get("runs")
        .get("steps")
        .to_vec()
        .iter()
        .filter_map(|step| {
            let name = step.find("name").map_or("a step", Node::scalar).to_string();
            step.find("run").map(|run| (name, run.scalar().to_string()))
        })
        .collect();
    scripts.push((
        "scripts/action/comment.sh".to_string(),
        read("scripts/action/comment.sh"),
    ));
    scripts
}

#[test]
fn no_array_expansion_breaks_on_the_bash_a_macos_runner_ships() {
    for (name, script) in action_scripts() {
        assert_eq!(
            unguarded_array_expansions(&script),
            Vec::<String>::new(),
            "`{name}` expands an array in a form bash 3.2 rejects under `set -u` \
             when it is empty; write it as ${{name[@]+\"${{name[@]}}\"}}"
        );
    }
}

/// The recognizer above, on the two forms it has to tell apart.
#[test]
fn the_guarded_array_form_is_the_only_one_that_passes() {
    assert_eq!(
        unguarded_array_expansions(r#"cmd ${paths[@]+"${paths[@]}"} > out"#),
        Vec::<String>::new()
    );
    assert_eq!(
        unguarded_array_expansions(r#"cmd "${paths[@]}" "${other[*]}""#),
        vec!["${paths[@]}", "${other[*]}"]
    );
    // A guard that names a different array is not a guard.
    assert_eq!(
        unguarded_array_expansions(r#"cmd ${paths[@]+"${other[@]}"}"#),
        vec!["${paths[@]}", "${other[@]}"]
    );
    // `$@` is special-cased by every bash, and a subscript that is not an
    // expansion at all is left alone.
    assert_eq!(
        unguarded_array_expansions(r#"cmd "$@" # see foo[@]"#),
        Vec::<String>::new()
    );
}

/// The two ways the binary can arrive, and the one guarantee each owes: `local`
/// builds the source that ships with the action (which is what makes a pull
/// request here test its own code), and every other value goes through the
/// checked-in installer that verifies what it downloads.
#[test]
fn the_install_step_builds_from_source_or_installs_a_verified_release() {
    let action = action();
    let install = action
        .get("runs")
        .get("steps")
        .list()
        .iter()
        .find(|step| step.find("id").is_some_and(|id| id.scalar() == "install"))
        .expect("the composite has an install step")
        .clone();
    let script = install.get("run").scalar().to_string();

    assert!(
        script.contains(r#"cargo install --path "$GITHUB_ACTION_PATH""#),
        "`version: local` must build the action's own checkout: {script}"
    );
    assert!(
        script.contains(r#""$GITHUB_ACTION_PATH/scripts/install.sh""#),
        "a released version must arrive through the checked-in installer: {script}"
    );
    assert!(
        repo_root().join("scripts/install.sh").exists(),
        "the installer the action calls is missing"
    );
    assert_eq!(
        install.get("env").get("VERSION").scalar(),
        "${{ inputs.version }}",
        "the version input must reach the script through the environment"
    );
}

/// The security rule this action lives under: a fork's branch name, title, or
/// body is attacker-controlled text, and `${{ }}` splices it into the script
/// *before* the shell sees it. Everything reaches a step through `env` instead.
#[test]
fn no_untrusted_event_value_is_interpolated_into_a_script() {
    for file in ["action.yml", ".github/workflows/notignored.yml"] {
        let text = read(file);
        let parsed = parse(&text);
        let steps: Vec<Node> = match file {
            "action.yml" => parsed.get("runs").get("steps").list().to_vec(),
            _ => parsed
                .get("jobs")
                .get("suppressions")
                .get("steps")
                .list()
                .to_vec(),
        };
        for step in run_steps(&steps) {
            let script = step.get("run").scalar();
            assert!(
                !script.contains("github.event"),
                "{file} interpolates an event value into a script: {script}"
            );
            assert!(
                !script.contains("inputs."),
                "{file} interpolates an input into a script; pass it through `env`: {script}"
            );
        }
    }
}

/// The action finds its own comment by the marker the renderer writes; drift
/// between the two turns every run into a fresh comment.
#[test]
fn the_comment_script_looks_for_the_marker_the_renderer_writes() {
    let script = read("scripts/action/comment.sh");
    let recorded = script
        .lines()
        .find_map(|line| line.trim().strip_prefix("# STICKY_MARKER:"))
        .map(str::trim)
        .expect("comment.sh records a STICKY_MARKER marker");
    assert_eq!(
        recorded,
        notignored::cli::MARKER,
        "comment.sh and the markdown renderer disagree about the sticky marker"
    );
    assert!(
        script.contains(&format!("MARKER='{recorded}'")),
        "comment.sh does not search for the marker it records"
    );
}

#[test]
fn the_dogfood_workflow_runs_this_repositorys_own_action_on_its_own_pull_requests() {
    let workflow = parse(&read(".github/workflows/notignored.yml"));
    assert!(
        workflow.get("on").find("pull_request").is_some(),
        "the dogfood workflow no longer runs on pull requests"
    );
    // Upserting a comment is a write; reading the diff is not.
    let permissions = workflow.get("permissions");
    assert_eq!(permissions.get("contents").scalar(), "read");
    assert_eq!(permissions.get("pull-requests").scalar(), "write");

    let steps = workflow
        .get("jobs")
        .get("suppressions")
        .get("steps")
        .to_vec();
    let checkout = steps
        .iter()
        .find(|step| {
            step.find("uses")
                .is_some_and(|uses| uses.scalar().starts_with("actions/checkout"))
        })
        .expect("the workflow checks the repository out");
    assert_eq!(
        checkout.get("with").get("fetch-depth").scalar(),
        "0",
        "--diff needs the base branch, which a shallow checkout does not fetch"
    );

    let action = steps
        .iter()
        .find(|step| step.find("uses").is_some_and(|uses| uses.scalar() == "./"))
        .expect("the workflow uses the action from this checkout");
    assert_eq!(
        action.get("with").get("version").scalar(),
        "local",
        "dogfooding must build the branch's own source, not a published release"
    );
}

/// A helper for the assertions above: sequences come back borrowed.
trait ToVec {
    fn to_vec(&self) -> Vec<Node>;
}

impl ToVec for Node {
    fn to_vec(&self) -> Vec<Node> {
        self.list().to_vec()
    }
}
