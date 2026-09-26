//! Recognises the check commands an agent ran (tests, builds, type checks, linters) and reads their
//! outcome. The runner's own summary lines decide first; the exit status is trusted only when the
//! check is the last thing the command ran, so `pnpm test | tail -5` can never read as a pass.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckOutcome {
    Pass,
    Fail,
    Unknown,
}

const SCANNED_BYTES: usize = 256 * 1024;

static FAILURES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"test result: FAILED",
        r"(?:^|[^\d.\w])[1-9]\d* (?:failed|failing)\b",
        r"error\[E\d{4}\]",
        r"error: could not compile",
        r"error TS\d+:",
        r"(?m)^\s*FAIL\s",
        r"(?m)^--- FAIL:",
        r"(?m)^FAILED\s",
        r"(?m)^ERROR\s",
        r"(?:^|[^\d.\w])[1-9]\d* errors? in [\d.]+s",
        r"ERR_PNPM_\w*FAIL",
        r"\bELIFECYCLE\b",
        r"(?m)^npm (?:ERR!|error) ",
        r"error during build",
        r"(?m)^error(?:\[E\d{4}\])?: ",
        r"Build FAILED",
        r"Failed!\s+-\s+Failed:\s+[1-9]",
        r"Command failed with exit code [1-9]",
        r"Tests Passed: \d+, Failed: [1-9]",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("valid failure pattern"))
    .collect()
});

static SUCCESSES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"test result: ok\.",
        r"(?m)^\s*Tests?\s+\d+ passed",
        r"(?m)^\s*Test Files\s+\d+ passed",
        r"=+ \d+ passed",
        r"(?m)^\s*\d+ passed",
        r"(?m)^Tests:\s+\d+ passed",
        r"Finished `[\w-]+` profile",
        r"✓ built in",
        r"Success: no issues found",
        r"All checks passed!",
        r"(?m)^ok\s+\S+\s+[\d.]+s",
        r"(?m)^\s*\d+ passing",
        r"Build succeeded",
        r"Passed!\s+-\s+Failed:\s+0",
        r"\bFound 0 errors\b",
        r"Tests Passed: \d+, Failed: 0",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("valid success pattern"))
    .collect()
});

static FOUND_ERRORS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bFound ([1-9]\d*) errors?(?: \((\d+) fixed, (\d+) remaining\))?")
        .expect("valid found-errors pattern")
});

static ESLINT_ERRORS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"✖ \d+ problems? \(([1-9]\d*) errors?").expect("valid eslint pattern")
});

const PACKAGE_MANAGERS: &[&str] = &["pnpm", "npm", "yarn", "bun", "deno"];
const PACKAGE_RUNNERS: &[&str] = &["npx", "pnpx", "bunx"];
const WRAPPERS: &[&str] = &["env", "time", "command", "builtin", "exec", "sudo", "nice"];
const SCRIPT_RUNNERS: &[&str] = &["uv", "poetry", "pipenv", "rye", "pdm", "hatch"];
const DIRECT_CHECKS: &[&str] = &[
    "jest", "mocha", "ava", "pytest", "py.test", "tsc", "mypy", "pyright", "eslint", "phpunit",
    "rspec", "ctest", "nextest",
];
const SCRIPT_CHECKS: &[&str] = &[
    "test",
    "tests",
    "typecheck",
    "type-check",
    "tsc",
    "lint",
    "check",
    "build",
    "verify",
    "vitest",
    "jest",
    "e2e",
    "ci",
];
const SCRIPT_CHECK_PREFIXES: &[&str] = &[
    "test:",
    "tests:",
    "check:",
    "lint:",
    "build:",
    "typecheck:",
    "verify:",
    "e2e:",
];
const RUNNER_FLAGS_WITH_VALUE: &[&str] = &[
    "--with",
    "--python",
    "-p",
    "--project",
    "--directory",
    "--package",
    "--extra",
    "--group",
    "--env-file",
];
const MANAGER_FLAGS_WITH_VALUE: &[&str] = &[
    "--filter",
    "-F",
    "-C",
    "--dir",
    "--prefix",
    "--workspace",
    "-w",
    "--cwd",
    "--config",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Separator {
    And,
    Or,
    Sequence,
    Pipe,
}

/// The outcome of a command that ran at least one check, or `None` when it ran no check.
pub(crate) fn check_outcome(
    command: &str,
    output: &str,
    exit_ok: Option<bool>,
) -> Option<CheckOutcome> {
    let segments = segments(command);
    let last_check = segments
        .iter()
        .rposition(|(segment, _)| is_check(segment))?;
    let text = clip(output);
    if failed(text) {
        return Some(CheckOutcome::Fail);
    }
    if SUCCESSES.iter().any(|pattern| pattern.is_match(text)) {
        return Some(CheckOutcome::Pass);
    }
    let runs_last = last_check + 1 == segments.len() && segments[last_check].1.is_none();
    Some(match (runs_last, exit_ok) {
        (true, Some(true)) => CheckOutcome::Pass,
        (true, Some(false)) => CheckOutcome::Fail,
        _ => CheckOutcome::Unknown,
    })
}

/// Whether the command runs a check at all, whatever its outcome.
#[cfg(test)]
fn runs_check(command: &str) -> bool {
    segments(command)
        .iter()
        .any(|(segment, _)| is_check(segment))
}

fn failed(text: &str) -> bool {
    if FAILURES.iter().any(|pattern| pattern.is_match(text)) {
        return true;
    }
    if ESLINT_ERRORS.is_match(text) {
        return true;
    }
    FOUND_ERRORS
        .captures_iter(text)
        .any(|found| match found.get(3) {
            Some(remaining) => remaining.as_str() != "0",
            None => true,
        })
}

fn clip(output: &str) -> &str {
    if output.len() <= SCANNED_BYTES {
        return output;
    }
    let mut start = output.len() - SCANNED_BYTES;
    while !output.is_char_boundary(start) {
        start += 1;
    }
    &output[start..]
}

/// Top-level command segments with the separator that follows each one. Quoted text and heredoc
/// bodies never split a segment, so a script piped into `python -` is not read as commands.
fn segments(command: &str) -> Vec<(String, Option<Separator>)> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut heredoc: Option<String> = None;
    let mut pending_heredoc: Option<String> = None;
    let chars: Vec<char> = command.chars().collect();
    let mut index = 0;
    let push = |result: &mut Vec<(String, Option<Separator>)>,
                current: &mut String,
                separator: Option<Separator>| {
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            result.push((trimmed.to_owned(), separator));
        }
        current.clear();
    };
    while index < chars.len() {
        let character = chars[index];
        if let Some(terminator) = &heredoc {
            let end = chars[index..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(chars.len(), |p| index + p);
            let line: String = chars[index..end].iter().collect();
            if line.trim() == terminator {
                heredoc = None;
            }
            index = end + 1;
            continue;
        }
        if let Some(open) = quote {
            current.push(character);
            if character == '\\' && open == '"' && index + 1 < chars.len() {
                current.push(chars[index + 1]);
                index += 2;
                continue;
            }
            if character == open {
                quote = None;
            }
            index += 1;
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                current.push(character);
                index += 1;
            }
            '<' if chars.get(index + 1) == Some(&'<') && chars.get(index + 2) != Some(&'<') => {
                let mut cursor = index + 2;
                if chars.get(cursor) == Some(&'-') {
                    cursor += 1;
                }
                while chars.get(cursor).is_some_and(|c| *c == ' ') {
                    cursor += 1;
                }
                let quoted = chars
                    .get(cursor)
                    .copied()
                    .filter(|c| *c == '\'' || *c == '"');
                if quoted.is_some() {
                    cursor += 1;
                }
                let start = cursor;
                while chars
                    .get(cursor)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    cursor += 1;
                }
                if cursor > start {
                    pending_heredoc = Some(chars[start..cursor].iter().collect());
                    if quoted.is_some() && chars.get(cursor) == quoted.as_ref() {
                        cursor += 1;
                    }
                    current.extend(chars[index..cursor].iter());
                    index = cursor;
                } else {
                    current.push(character);
                    index += 1;
                }
            }
            '\n' => {
                push(&mut result, &mut current, Some(Separator::Sequence));
                if let Some(terminator) = pending_heredoc.take() {
                    heredoc = Some(terminator);
                }
                index += 1;
            }
            ';' => {
                push(&mut result, &mut current, Some(Separator::Sequence));
                index += 1;
            }
            '&' if chars.get(index + 1) == Some(&'&') => {
                push(&mut result, &mut current, Some(Separator::And));
                index += 2;
            }
            '|' if chars.get(index + 1) == Some(&'|') => {
                push(&mut result, &mut current, Some(Separator::Or));
                index += 2;
            }
            '|' => {
                push(&mut result, &mut current, Some(Separator::Pipe));
                index += 1;
            }
            _ => {
                current.push(character);
                index += 1;
            }
        }
    }
    push(&mut result, &mut current, None);
    if let Some(last) = result.last_mut()
        && last.1 == Some(Separator::Sequence)
    {
        last.1 = None;
    }
    result
}

fn words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for character in segment.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => current.push(character),
            None if character == '\'' || character == '"' => quote = Some(character),
            None if character.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            None => current.push(character),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn program(word: &str) -> String {
    let name = word
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(word)
        .to_ascii_lowercase();
    for extension in [".exe", ".cmd", ".bat", ".ps1", ".js"] {
        if let Some(stem) = name.strip_suffix(extension) {
            return stem.to_owned();
        }
    }
    name
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let name = name.strip_prefix("$env:").unwrap_or(name);
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_check(segment: &str) -> bool {
    let words = words(segment.trim_start_matches(['(', '{', '!', ' ']));
    check_words(&words)
}

fn check_words(words: &[String]) -> bool {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        let name = program(word);
        if word == "&" || is_assignment(word) || WRAPPERS.contains(&name.as_str()) {
            index += 1;
            continue;
        }
        if name == "timeout" {
            index += 2;
            continue;
        }
        break;
    }
    let Some(head) = words.get(index) else {
        return false;
    };
    let head = program(head);
    let rest = &words[index + 1..];
    let arguments = || {
        rest.iter()
            .filter(|word| !word.starts_with('-') && !word.starts_with('+'))
    };
    let has = |flag: &str| rest.iter().any(|word| word == flag);
    match head.as_str() {
        "cargo" => arguments().next().is_some_and(|sub| {
            matches!(
                sub.as_str(),
                "test" | "build" | "check" | "clippy" | "nextest"
            ) || (sub == "fmt" && has("--check"))
        }),
        name if PACKAGE_MANAGERS.contains(&name) => package_script(rest),
        name if PACKAGE_RUNNERS.contains(&name) => {
            let tool = rest.iter().position(|word| !word.starts_with('-'));
            tool.is_some_and(|position| check_words(&rest[position..]))
        }
        name if SCRIPT_RUNNERS.contains(&name) => {
            rest.first().is_some_and(|word| word == "run")
                && check_words(skip_flags(&rest[1..], RUNNER_FLAGS_WITH_VALUE))
        }
        "vitest" => !has("--watch") && !rest.first().is_some_and(|word| word == "watch"),
        name if DIRECT_CHECKS.contains(&name) => !has("--watch") && !has("--version"),
        "ruff" => has("check") || (has("format") && has("--check")),
        "prettier" => has("--check"),
        "python" | "python3" | "py" => rest
            .iter()
            .position(|word| word == "-m")
            .and_then(|position| rest.get(position + 1))
            .is_some_and(|module| {
                matches!(
                    module.as_str(),
                    "pytest" | "unittest" | "mypy" | "pyright" | "compileall" | "py_compile"
                ) || (module == "ruff" && has("check"))
            }),
        "go" => arguments()
            .next()
            .is_some_and(|sub| matches!(sub.as_str(), "test" | "build" | "vet")),
        "dotnet" => arguments()
            .next()
            .is_some_and(|sub| matches!(sub.as_str(), "test" | "build")),
        "mvn" | "mvnw" | "gradle" | "gradlew" => {
            arguments().any(|word| matches!(word.as_str(), "test" | "verify" | "build" | "check"))
        }
        "playwright" => arguments().next().is_some_and(|sub| sub == "test"),
        "make" => arguments()
            .any(|word| matches!(word.as_str(), "test" | "check" | "lint" | "build" | "ci")),
        "node" => has("--test"),
        _ => false,
    }
}

fn skip_flags<'a>(words: &'a [String], with_value: &[&str]) -> &'a [String] {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if with_value.contains(&word.as_str()) {
            index += 2;
        } else if word.starts_with('-') {
            index += 1;
        } else {
            break;
        }
    }
    &words[index.min(words.len())..]
}

fn package_script(rest: &[String]) -> bool {
    let rest = skip_flags(rest, MANAGER_FLAGS_WITH_VALUE);
    let Some(sub) = rest.first() else {
        return false;
    };
    match sub.as_str() {
        "exec" | "x" | "dlx" => check_words(skip_flags(&rest[1..], MANAGER_FLAGS_WITH_VALUE)),
        "run" | "run-script" => skip_flags(&rest[1..], MANAGER_FLAGS_WITH_VALUE)
            .first()
            .is_some_and(|script| check_script(script)),
        script => check_script(script),
    }
}

fn check_script(script: &str) -> bool {
    let script = script.to_ascii_lowercase();
    SCRIPT_CHECKS.contains(&script.as_str())
        || SCRIPT_CHECK_PREFIXES
            .iter()
            .any(|prefix| script.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_check_commands_across_toolchains() {
        for command in [
            "cargo test -p uc-logscan",
            "cargo +nightly clippy --all-targets -- -D warnings",
            "cargo fmt --check",
            "cd \"C:/work/demo app\" && pnpm --filter @demo/web typecheck",
            "pnpm -F web exec vitest run src/a.test.ts",
            "pnpm run test:unit",
            "pnpm check:internal-documents",
            "npm run lint",
            "npx.cmd vitest run --project server",
            "npx tsc --noEmit -p tsconfig.json",
            "python -m pytest tests/ -q",
            "./backend/.venv/Scripts/python.exe -m pytest backend/tests",
            "uv run --frozen ruff check src",
            "uv run pytest",
            "PYTHONIOENCODING=utf-8 python -m pytest tests/test_x.py",
            "timeout 1800 ./node_modules/.bin/playwright test e2e/an-toan.spec.ts",
            "go test ./...",
            "dotnet build",
            "& \"C:\\Program Files\\nodejs\\npm.cmd\" test",
            "$env:CI=1; pnpm test",
        ] {
            assert!(runs_check(command), "{command}");
        }
        for command in [
            "git status --short",
            "grep -n \"pnpm test\" README.md",
            "echo \"cargo test later\"",
            "pnpm install --frozen-lockfile",
            "npm view @demo/cli version",
            "cargo run --example usage_ledger",
            "python - <<'PY'\nimport subprocess\nsubprocess.run(['cargo','test'])\npytest\nPY",
            "ruff format src",
            "pnpm dev",
            "vitest --watch",
        ] {
            assert!(!runs_check(command), "{command}");
        }
    }

    #[test]
    fn summary_lines_decide_before_the_exit_status() {
        let piped = "cd repo && pnpm test 2>&1 | tail -5";
        assert_eq!(
            check_outcome(
                piped,
                " Test Files  1 failed (1)\n      Tests  2 failed | 10 passed (12)",
                Some(true)
            ),
            Some(CheckOutcome::Fail)
        );
        assert_eq!(
            check_outcome(
                piped,
                " Test Files  3 passed (3)\n      Tests  137 passed (137)",
                Some(true)
            ),
            Some(CheckOutcome::Pass)
        );
        assert_eq!(
            check_outcome(piped, "done", Some(true)),
            Some(CheckOutcome::Unknown)
        );
        assert_eq!(
            check_outcome(
                "cargo test 2>&1 | tail -3",
                "test result: FAILED. 3 passed; 1 failed",
                Some(true)
            ),
            Some(CheckOutcome::Fail)
        );
        assert_eq!(
            check_outcome(
                "python -m pytest -q 2>&1 | tail -2",
                "3265 passed, 21 skipped in 80.72s",
                Some(true)
            ),
            Some(CheckOutcome::Pass)
        );
        assert_eq!(
            check_outcome(
                "npx tsc --noEmit 2>&1 | head -30",
                "src/a.ts(1,2): error TS2307: Cannot find module",
                Some(true)
            ),
            Some(CheckOutcome::Fail)
        );
    }

    #[test]
    fn exit_status_counts_only_when_the_check_ran_last() {
        assert_eq!(
            check_outcome("cd web && npx tsc --noEmit", "", Some(true)),
            Some(CheckOutcome::Pass)
        );
        assert_eq!(
            check_outcome("cd web && npx tsc --noEmit", "", Some(false)),
            Some(CheckOutcome::Fail)
        );
        assert_eq!(
            check_outcome("pnpm test || true", "", Some(true)),
            Some(CheckOutcome::Unknown)
        );
        assert_eq!(
            check_outcome("pnpm lint; echo done", "done", Some(true)),
            Some(CheckOutcome::Unknown)
        );
        assert_eq!(
            check_outcome("pnpm lint", "", None),
            Some(CheckOutcome::Unknown)
        );
        assert_eq!(check_outcome("git status", "", Some(false)), None);
    }

    #[test]
    fn fixed_lint_errors_and_warnings_are_not_failures() {
        assert_eq!(
            check_outcome(
                "ruff check --fix src",
                "Found 1 error (1 fixed, 0 remaining).\nAll checks passed!",
                Some(true)
            ),
            Some(CheckOutcome::Pass)
        );
        assert_eq!(
            check_outcome(
                "ruff check src | tail -2",
                "Found 5 errors (1 fixed, 4 remaining).",
                Some(true)
            ),
            Some(CheckOutcome::Fail)
        );
        assert_eq!(
            check_outcome(
                "npm run lint 2>&1 | tail -3",
                "✖ 2 problems (0 errors, 2 warnings)",
                Some(true)
            ),
            Some(CheckOutcome::Unknown)
        );
        assert_eq!(
            check_outcome(
                "npm run lint 2>&1 | tail -3",
                "✖ 2 problems (1 error, 1 warning)",
                Some(true)
            ),
            Some(CheckOutcome::Fail)
        );
        assert_eq!(
            check_outcome("pnpm test", "0 failed", Some(true)),
            Some(CheckOutcome::Pass)
        );
        assert_eq!(
            check_outcome("pnpm test | tail", "0 failed", Some(true)),
            Some(CheckOutcome::Unknown)
        );
    }

    #[test]
    fn heredoc_bodies_and_quotes_do_not_split_commands() {
        let command = "cd x && python - <<'PY'\nprint('a && b | c')\nPY\npnpm test";
        let parts = segments(command);
        assert_eq!(
            parts
                .last()
                .map(|(segment, separator)| (segment.as_str(), *separator)),
            Some(("pnpm test", None))
        );
        assert!(parts.iter().all(|(segment, _)| !segment.contains("print")));
        assert_eq!(segments("echo 'a; b' && cargo test").len(), 2);
    }

    #[test]
    fn long_outputs_are_read_from_their_tail() {
        let output = format!(
            "{}\ntest result: FAILED. 1 passed; 1 failed",
            "x".repeat(SCANNED_BYTES)
        );
        assert_eq!(
            check_outcome("cargo test", &output, Some(true)),
            Some(CheckOutcome::Fail)
        );
    }
}
