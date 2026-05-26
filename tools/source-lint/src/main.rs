use std::{
    env,
    ffi::{OsStr, OsString},
    fmt, fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
};

use proc_macro2::Span;
use syn::{
    Attribute, ExprLit, File, ImplItemConst, Item, ItemConst, ItemImpl, ItemMod, ItemStatic,
    ItemUse, Lit, Meta, TraitItemConst, Type, UseTree, Visibility,
    visit::{self, Visit},
};

type Result<T> = std::result::Result<T, LintError>;

const DEFAULT_ROOT: &str = "src";
const REPOSITORY_ROOT: &str = ".";
const RUST_EXTENSION: &str = "rs";
const MARKDOWN_EXTENSION: &str = "md";
const SKIPPED_DIRECTORIES: [&str; 3] = [".git", ".cyanos", "target"];
const ALLOWED_INTEGER_LITERALS: [&str; 2] = ["0", "1"];
const MODULE_BOUNDARY_FILE: &str = "mod.rs";
const LIBRARY_ROOT_FILE: &str = "lib.rs";
const MAIN_ROOT_FILE: &str = "main.rs";
const AGENT_IDENTITY_FILE: &str = "src/agent/identity.rs";
const AGENT_SUBJECT_TOKEN: &str = "Agent";
const STRICT_SUBJECT_ROOTS: [&str; 1] = ["src/agent/"];
const ARCHITECTURE_AGENT_TOKENS: [&str; 7] = [
    "Claude",
    "Codex",
    "Gemini",
    "adapters::",
    "Agent::Claude",
    "Agent::Codex",
    "Agent::Gemini",
];
const LOOP_EVAL_FORBIDDEN_TOKENS: [&str; 6] = [
    "ModelSelection",
    "AgentRequest",
    "CommandSpec",
    "AdapterRegistry",
    "--agent",
    "--model",
];
const SEMANTIC_STRING_TOKENS: [&str; 42] = [
    "--agent",
    "--model",
    "--path",
    "--project",
    "--version",
    ".md",
    ".json",
    ".jsonl",
    ".lock",
    "README",
    "result",
    "summary",
    "auth",
    "best",
    "best-supported",
    "claude",
    "codex",
    "demo",
    "eval",
    "exec",
    "feature",
    "gemini",
    "gh",
    "git",
    "init",
    "ledger",
    "lineages",
    "login",
    "origin",
    "program",
    "prompt",
    "projects",
    "cyanos",
    "run",
    "runs",
    "samples",
    "status",
    "task-",
    "tasks",
    "verification",
    "verifier",
    "worktree",
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let command = LintCommand::from_env();
    let mut violations = Vec::new();
    command.lint(&mut violations)?;

    if violations.is_empty() {
        return Ok(());
    }

    Err(LintError::Violations(violations))
}

trait LintPolicy {
    fn lint(&self, violations: &mut Vec<Violation>) -> Result<()>;
}

#[derive(Debug)]
enum LintCommand {
    Source(SourcePolicy),
    Architecture(ArchitecturePolicy),
    DocsLanguage(DocsLanguagePolicy),
}

impl LintCommand {
    fn from_env() -> Self {
        Self::from_args(env::args_os().skip(1).collect())
    }

    fn from_args(mut args: Vec<OsString>) -> Self {
        if matches!(
            args.first().and_then(|argument| argument.to_str()),
            Some("source")
        ) {
            drop(args.remove(0));
            return Self::Source(SourcePolicy::new(paths_or_default(&args, DEFAULT_ROOT)));
        }

        if matches!(
            args.first().and_then(|argument| argument.to_str()),
            Some("architecture")
        ) {
            drop(args.remove(0));
            return Self::Architecture(ArchitecturePolicy::new(paths_or_default(
                &args,
                DEFAULT_ROOT,
            )));
        }

        if matches!(
            args.first().and_then(|argument| argument.to_str()),
            Some("docs-language")
        ) {
            return Self::DocsLanguage(DocsLanguagePolicy::new(PathBuf::from(REPOSITORY_ROOT)));
        }

        Self::Source(SourcePolicy::new(paths_or_default(&args, DEFAULT_ROOT)))
    }

    fn lint(&self, violations: &mut Vec<Violation>) -> Result<()> {
        match self {
            Self::Source(policy) => policy.lint(violations),
            Self::Architecture(policy) => policy.lint(violations),
            Self::DocsLanguage(policy) => policy.lint(violations),
        }
    }
}

#[derive(Debug)]
struct SourcePolicy {
    roots: Vec<PathBuf>,
}

impl SourcePolicy {
    fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }
}

impl LintPolicy for SourcePolicy {
    fn lint(&self, violations: &mut Vec<Violation>) -> Result<()> {
        let mut files = Vec::new();
        for root in &self.roots {
            collect_files_with_extension(root, RUST_EXTENSION, &mut files)?;
        }
        files.sort();

        for file in files {
            lint_source_file(&file, violations)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct ArchitecturePolicy {
    roots: Vec<PathBuf>,
}

impl ArchitecturePolicy {
    fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }
}

impl LintPolicy for ArchitecturePolicy {
    fn lint(&self, violations: &mut Vec<Violation>) -> Result<()> {
        let mut files = Vec::new();
        for root in &self.roots {
            collect_files_with_extension(root, RUST_EXTENSION, &mut files)?;
        }
        files.sort();

        for file in files {
            lint_architecture_file(&file, violations)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct DocsLanguagePolicy {
    root: PathBuf,
}

impl DocsLanguagePolicy {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl LintPolicy for DocsLanguagePolicy {
    fn lint(&self, violations: &mut Vec<Violation>) -> Result<()> {
        let mut files = Vec::new();
        collect_files_with_extension(&self.root, MARKDOWN_EXTENSION, &mut files)?;
        files.sort();

        for file in files {
            lint_docs_language_file(&file, violations)?;
        }

        Ok(())
    }
}

fn paths_or_default(args: &[OsString], default: &str) -> Vec<PathBuf> {
    if args.is_empty() {
        vec![PathBuf::from(default)]
    } else {
        args.iter().map(PathBuf::from).collect()
    }
}

fn collect_files_with_extension(
    path: &Path,
    extension: &str,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_file() {
        if path.extension() == Some(OsStr::new(extension)) {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }

    if should_skip_directory(path) {
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|source| LintError::Io {
        action: "read directory",
        path: path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| LintError::Io {
            action: "read directory entry",
            path: path.to_path_buf(),
            source,
        })?;
        collect_files_with_extension(&entry.path(), extension, files)?;
    }

    Ok(())
}

fn should_skip_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| SKIPPED_DIRECTORIES.contains(&name))
}

fn lint_source_file(path: &Path, violations: &mut Vec<Violation>) -> Result<()> {
    let source = read_text(path, "read source file")?;
    let syntax = syn::parse_file(&source).map_err(|source| LintError::Parse {
        path: path.to_path_buf(),
        source,
    })?;

    lint_file_subject(path, &syntax, violations);

    let mut visitor = LiteralVisitor { path, violations };
    visitor.visit_file(&syntax);

    Ok(())
}

fn lint_file_subject(path: &Path, syntax: &File, violations: &mut Vec<Violation>) {
    let subjects = top_level_subjects(syntax);

    if is_module_boundary_file(path) && !subjects.is_empty() {
        push_subject_violation(
            path,
            &subjects[0],
            "module boundary files may only declare modules and re-exports; move concrete subjects into their own files",
            violations,
        );
    }

    if is_plural_subject_file(path) && subjects.len() > 1 {
        push_subject_violation(
            path,
            &subjects[1],
            "plural source files cannot own multiple subjects; split each subject into its own file",
            violations,
        );
    }

    if is_strict_subject_file(path) {
        lint_strict_subject_file(path, &subjects, violations);
    }

    lint_adapter_impl_subjects(path, syntax, violations);
}

fn lint_strict_subject_file(
    path: &Path,
    subjects: &[SubjectItem],
    violations: &mut Vec<Violation>,
) {
    let Some(expected) = subject_token(path) else {
        return;
    };

    for subject in subjects {
        if !subject.name.contains(&expected) {
            push_subject_violation(
                path,
                subject,
                format!(
                    "subject `{}` does not match file subject `{expected}`; move it into its own file",
                    subject.name
                ),
                violations,
            );
        }
    }
}

fn lint_adapter_impl_subjects(path: &Path, syntax: &File, violations: &mut Vec<Violation>) {
    let adapter_impls = adapter_impl_targets(syntax);
    if adapter_impls.len() <= 1 {
        return;
    }

    for subject in adapter_impls.iter().skip(1) {
        push_subject_violation(
            path,
            subject,
            "a file may implement Adapter for only one concrete agent; split adapters by agent",
            violations,
        );
    }
}

#[derive(Clone, Debug)]
struct SubjectItem {
    name: String,
    span: Span,
}

fn top_level_subjects(syntax: &File) -> Vec<SubjectItem> {
    syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(item) => Some(SubjectItem::new(item.ident.to_string(), item.ident.span())),
            Item::Enum(item) => Some(SubjectItem::new(item.ident.to_string(), item.ident.span())),
            Item::Trait(item) => Some(SubjectItem::new(item.ident.to_string(), item.ident.span())),
            _ => None,
        })
        .collect()
}

fn adapter_impl_targets(syntax: &File) -> Vec<SubjectItem> {
    syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(item) => adapter_impl_target(item),
            _ => None,
        })
        .collect()
}

fn adapter_impl_target(item: &ItemImpl) -> Option<SubjectItem> {
    let (_, trait_path, _) = item.trait_.as_ref()?;
    let segment = trait_path.segments.last()?;
    if segment.ident != "Adapter" {
        return None;
    }

    let Type::Path(target) = item.self_ty.as_ref() else {
        return None;
    };
    let target = target.path.segments.last()?;

    Some(SubjectItem::new(
        target.ident.to_string(),
        item.impl_token.span,
    ))
}

impl SubjectItem {
    fn new(name: String, span: Span) -> Self {
        Self { name, span }
    }
}

fn push_subject_violation(
    path: &Path,
    subject: &SubjectItem,
    message: impl Into<String>,
    violations: &mut Vec<Violation>,
) {
    let start = subject.span.start();
    violations.push(Violation::new(
        path,
        start.line,
        start.column + 1,
        LintKind::FileSubject,
        message,
    ));
}

fn is_module_boundary_file(path: &Path) -> bool {
    file_name(path).is_some_and(|name| name == MODULE_BOUNDARY_FILE)
}

fn is_plural_subject_file(path: &Path) -> bool {
    file_stem(path).is_some_and(|stem| stem.ends_with('s')) && !is_root_file(path)
}

fn is_strict_subject_file(path: &Path) -> bool {
    if is_root_file(path) || is_module_boundary_file(path) {
        return false;
    }

    let normalized = normalize_path(path);
    STRICT_SUBJECT_ROOTS
        .iter()
        .any(|root| normalized.starts_with(root) || normalized.contains(&format!("/{root}")))
}

fn is_root_file(path: &Path) -> bool {
    file_name(path).is_some_and(|name| name == LIBRARY_ROOT_FILE || name == MAIN_ROOT_FILE)
}

fn subject_token(path: &Path) -> Option<String> {
    if normalize_path(path).ends_with(AGENT_IDENTITY_FILE) {
        return Some(AGENT_SUBJECT_TOKEN.to_owned());
    }

    file_stem(path).map(|stem| {
        stem.split(['_', '-'])
            .filter(|part| !part.is_empty())
            .map(capitalize)
            .collect::<String>()
    })
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };

    first.to_uppercase().chain(characters).collect::<String>()
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(OsStr::to_str)
}

fn file_stem(path: &Path) -> Option<&str> {
    path.file_stem().and_then(OsStr::to_str)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn lint_architecture_file(path: &Path, violations: &mut Vec<Violation>) -> Result<()> {
    let source = read_text(path, "read architecture source file")?;

    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;

        if !is_inside_agent_layer(path) {
            if let Some(token) = first_matching_token(line, &ARCHITECTURE_AGENT_TOKENS) {
                violations.push(Violation::new(
                    path,
                    line_number,
                    token.column + 1,
                    LintKind::ArchitectureBoundary,
                    format!(
                        "concrete agent token `{}` must stay inside src/agent",
                        token.value
                    ),
                ));
            }
        }

        if is_loop_or_eval_file(path) {
            if let Some(token) = first_matching_token(line, &LOOP_EVAL_FORBIDDEN_TOKENS) {
                violations.push(Violation::new(
                    path,
                    line_number,
                    token.column + 1,
                    LintKind::ArchitectureBoundary,
                    format!(
                        "loop and eval layers may depend only on AgentRuntime, AgentTurn, and AgentOutput; move `{}` behind the agent boundary",
                        token.value
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn lint_docs_language_file(path: &Path, violations: &mut Vec<Violation>) -> Result<()> {
    let source = read_text(path, "read documentation file")?;

    for (index, line) in source.lines().enumerate() {
        if let Some(character) = find_non_english_letter(line) {
            violations.push(Violation::new(
                path,
                index + 1,
                character.column + 1,
                LintKind::DocsLanguage,
                format!(
                    "documentation must use English prose; replace non-English letter `{}`",
                    character.value
                ),
            ));
        }
    }

    Ok(())
}

fn read_text(path: &Path, action: &'static str) -> Result<String> {
    fs::read_to_string(path).map_err(|source| LintError::Io {
        action,
        path: path.to_path_buf(),
        source,
    })
}

fn is_inside_agent_layer(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized == "src/agent"
        || normalized.starts_with("src/agent/")
        || normalized.contains("/src/agent/")
}

fn is_loop_or_eval_file(path: &Path) -> bool {
    path.ends_with(Path::new("src/loop.rs")) || path.ends_with(Path::new("src/eval.rs"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TokenMatch {
    value: &'static str,
    column: usize,
}

fn first_matching_token(line: &str, tokens: &[&'static str]) -> Option<TokenMatch> {
    let mut best_match: Option<TokenMatch> = None;

    for token in tokens {
        if let Some(column) = line.find(token) {
            best_match = match best_match {
                Some(current) if current.column <= column => Some(current),
                _ => Some(TokenMatch {
                    value: token,
                    column,
                }),
            };
        }
    }

    best_match
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CharacterMatch {
    value: char,
    column: usize,
}

fn find_non_english_letter(line: &str) -> Option<CharacterMatch> {
    for (column, value) in line.chars().enumerate() {
        if value.is_alphabetic() && !value.is_ascii() {
            return Some(CharacterMatch { value, column });
        }
    }

    None
}

struct LiteralVisitor<'a> {
    path: &'a Path,
    violations: &'a mut Vec<Violation>,
}

impl<'ast> Visit<'ast> for LiteralVisitor<'_> {
    fn visit_attribute(&mut self, i: &'ast Attribute) {
        if i.path().is_ident("allow") {
            self.push_violation(
                i.bracket_token.span.open(),
                LintKind::LintOverride,
                "replace #[allow(...)] with #[expect(..., reason = \"...\")]",
            );
            return;
        }

        if i.path().is_ident("expect") && !has_reason(i) {
            self.push_violation(
                i.bracket_token.span.open(),
                LintKind::LintOverride,
                "add a reason to #[expect(...)]",
            );
            return;
        }

        visit::visit_attribute(self, i);
    }

    fn visit_item_const(&mut self, _node: &'ast ItemConst) {}

    fn visit_impl_item_const(&mut self, _node: &'ast ImplItemConst) {}

    fn visit_trait_item_const(&mut self, _node: &'ast TraitItemConst) {}

    fn visit_item_static(&mut self, i: &'ast ItemStatic) {
        self.push_violation(
            i.static_token.span,
            LintKind::StaticItem,
            "replace static state with owned runtime state; use const for immutable values",
        );
    }

    fn visit_item_mod(&mut self, i: &'ast ItemMod) {
        if has_cfg_test(&i.attrs) {
            return;
        }

        visit::visit_item_mod(self, i);
    }

    fn visit_item_use(&mut self, i: &'ast ItemUse) {
        if is_public_visibility(&i.vis) && contains_glob(&i.tree) {
            self.push_violation(
                i.use_token.span,
                LintKind::GlobReexport,
                "replace public glob re-export with explicit item re-exports",
            );
        }

        visit::visit_item_use(self, i);
    }

    fn visit_expr_lit(&mut self, i: &'ast ExprLit) {
        match &i.lit {
            Lit::Str(literal) => {
                let value = literal.value();
                if is_semantic_string_literal(&value) {
                    self.push_violation(
                        literal.span(),
                        LintKind::HardcodedString,
                        format!("move semantic string literal `{value}` into a named const"),
                    );
                }
            }
            Lit::ByteStr(literal) if !literal.value().is_empty() => {
                self.push_violation(
                    literal.span(),
                    LintKind::HardcodedString,
                    "move semantic byte string literal into a named const",
                );
            }
            Lit::Int(literal) => {
                let digits = literal
                    .base10_digits()
                    .trim_start_matches('-')
                    .trim_start_matches('+');
                if !ALLOWED_INTEGER_LITERALS.contains(&digits) {
                    self.push_violation(
                        literal.span(),
                        LintKind::MagicNumber,
                        format!("move numeric literal `{literal}` into a named const"),
                    );
                }
            }
            Lit::Float(literal) => {
                self.push_violation(
                    literal.span(),
                    LintKind::MagicNumber,
                    format!("move float literal `{literal}` into a named const"),
                );
            }
            _ => {}
        }

        visit::visit_expr_lit(self, i);
    }
}

impl LiteralVisitor<'_> {
    fn push_violation(&mut self, span: Span, kind: LintKind, message: impl Into<String>) {
        let start = span.start();
        self.violations.push(Violation::new(
            self.path,
            start.line,
            start.column + 1,
            kind,
            message,
        ));
    }
}

fn has_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && matches!(&attribute.meta, Meta::List(list) if list.tokens.to_string().contains("test"))
    })
}

fn has_reason(attribute: &Attribute) -> bool {
    match &attribute.meta {
        Meta::List(list) => list.tokens.to_string().contains("reason"),
        Meta::NameValue(_) | Meta::Path(_) => false,
    }
}

fn is_public_visibility(visibility: &Visibility) -> bool {
    matches!(
        visibility,
        Visibility::Public(_) | Visibility::Restricted(_)
    )
}

fn contains_glob(tree: &UseTree) -> bool {
    match tree {
        UseTree::Glob(_) => true,
        UseTree::Group(group) => group.items.iter().any(contains_glob),
        UseTree::Path(path) => contains_glob(&path.tree),
        UseTree::Name(_) | UseTree::Rename(_) => false,
    }
}

fn is_semantic_string_literal(value: &str) -> bool {
    if value.is_empty() || value.chars().count() == 1 || value.contains(' ') {
        return false;
    }

    SEMANTIC_STRING_TOKENS
        .iter()
        .any(|token| value == *token || value.contains(token))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LintKind {
    ArchitectureBoundary,
    DocsLanguage,
    FileSubject,
    GlobReexport,
    HardcodedString,
    LintOverride,
    MagicNumber,
    StaticItem,
}

impl fmt::Display for LintKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArchitectureBoundary => f.write_str("architecture-boundary"),
            Self::DocsLanguage => f.write_str("docs-language"),
            Self::FileSubject => f.write_str("file-subject"),
            Self::GlobReexport => f.write_str("glob-reexport"),
            Self::HardcodedString => f.write_str("hardcoded-string"),
            Self::LintOverride => f.write_str("lint-override"),
            Self::MagicNumber => f.write_str("magic-number"),
            Self::StaticItem => f.write_str("static-item"),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Violation {
    path: PathBuf,
    line: usize,
    column: usize,
    kind: LintKind,
    message: String,
}

impl Violation {
    fn new(
        path: impl Into<PathBuf>,
        line: usize,
        column: usize,
        kind: LintKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            line,
            column,
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}: {}",
            self.path.display(),
            self.line,
            self.column,
            self.kind,
            self.message
        )
    }
}

#[derive(Debug)]
enum LintError {
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: syn::Error,
    },
    Violations(Vec<Violation>),
}

impl fmt::Display for LintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} at {}: {source}", path.display()),
            Self::Parse { path, source } => {
                write!(f, "failed to parse {}: {source}", path.display())
            }
            Self::Violations(violations) => {
                writeln!(f, "cyanos lint failed: fix policy violations")?;
                for violation in violations {
                    writeln!(f, "{violation}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for LintError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Violations(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ARCHITECTURE_AGENT_TOKENS, CharacterMatch, LOOP_EVAL_FORBIDDEN_TOKENS, LintKind,
        TokenMatch, Violation, contains_glob, find_non_english_letter, first_matching_token,
        has_reason, is_inside_agent_layer, is_loop_or_eval_file, is_semantic_string_literal,
        lint_file_subject,
    };
    use std::path::{Path, PathBuf};
    use syn::{Attribute, File, ItemUse, parse_quote};

    #[test]
    fn classifies_semantic_string_literals() {
        assert!(is_semantic_string_literal("--project"));
        assert!(is_semantic_string_literal("README.md"));
        assert!(is_semantic_string_literal("cyanos/task"));
        assert!(!is_semantic_string_literal(""));
        assert!(!is_semantic_string_literal("x"));
        assert!(!is_semantic_string_literal("user-facing sentence"));
    }

    #[test]
    fn displays_violations() {
        let violation = Violation::new(
            PathBuf::from("src/main.rs"),
            3,
            7,
            LintKind::MagicNumber,
            "move numeric literal `2` into a named const",
        );

        assert_eq!(
            violation.to_string(),
            "src/main.rs:3:7: magic-number: move numeric literal `2` into a named const"
        );
    }

    #[test]
    fn detects_architecture_boundary_tokens() {
        assert_eq!(
            first_matching_token("Agent::Claude", &ARCHITECTURE_AGENT_TOKENS),
            Some(TokenMatch {
                value: "Agent::Claude",
                column: 0
            })
        );
        assert_eq!(
            first_matching_token("let request: AgentRequest;", &LOOP_EVAL_FORBIDDEN_TOKENS),
            Some(TokenMatch {
                value: "AgentRequest",
                column: 13
            })
        );
    }

    #[test]
    fn recognizes_loop_and_eval_files() {
        assert!(is_loop_or_eval_file(Path::new("src/loop.rs")));
        assert!(is_loop_or_eval_file(Path::new("/tmp/cyanos/src/eval.rs")));
        assert!(!is_loop_or_eval_file(Path::new("src/agent/runtime.rs")));
    }

    #[test]
    fn recognizes_agent_layer_files() {
        assert!(is_inside_agent_layer(Path::new("src/agent/runtime.rs")));
        assert!(is_inside_agent_layer(Path::new(
            "/tmp/cyanos/src/agent/runtime.rs"
        )));
        assert!(!is_inside_agent_layer(Path::new("src/loop.rs")));
    }

    #[test]
    fn detects_non_english_letters_but_allows_emoji() {
        assert_eq!(find_non_english_letter("English text 😀"), None);
        assert_eq!(
            find_non_english_letter("English 文"),
            Some(CharacterMatch {
                value: '文',
                column: 8
            })
        );
    }

    #[test]
    fn recognizes_public_glob_reexports() {
        let item: ItemUse = parse_quote!(
            pub use crate::foo::*;
        );

        assert!(contains_glob(&item.tree));
    }

    #[test]
    fn requires_expect_reason() {
        let with_reason: Attribute = parse_quote!(#[expect(clippy::panic, reason = "tested")]);
        let without_reason: Attribute = parse_quote!(#[expect(clippy::panic)]);

        assert!(has_reason(&with_reason));
        assert!(!has_reason(&without_reason));
    }

    #[test]
    fn rejects_concrete_subjects_in_module_boundary_files() {
        let file: File = parse_quote!(
            pub struct Registry;
        );
        let mut violations = Vec::new();

        lint_file_subject(Path::new("src/agent/mod.rs"), &file, &mut violations);

        assert!(violations.iter().any(|violation| {
            violation.kind == LintKind::FileSubject && violation.message.contains("module boundary")
        }));
    }

    #[test]
    fn rejects_multiple_subjects_in_plural_files() {
        let file: File = parse_quote!(
            pub struct Claude;
            pub struct Codex;
        );
        let mut violations = Vec::new();

        lint_file_subject(Path::new("src/agent/adapters.rs"), &file, &mut violations);

        assert!(violations.iter().any(|violation| {
            violation.kind == LintKind::FileSubject
                && violation.message.contains("multiple subjects")
        }));
    }

    #[test]
    fn rejects_multiple_adapter_impls_in_one_file() {
        let file: File = parse_quote!(
            pub struct Claude;
            pub struct Codex;

            impl Adapter for Claude {}
            impl Adapter for Codex {}
        );
        let mut violations = Vec::new();

        lint_file_subject(Path::new("src/agent/claude.rs"), &file, &mut violations);

        assert!(violations.iter().any(|violation| {
            violation.kind == LintKind::FileSubject
                && violation.message.contains("only one concrete agent")
        }));
    }

    #[test]
    fn rejects_subject_names_that_do_not_match_strict_file_name() {
        let file: File = parse_quote!(
            pub struct AgentRequest;
            pub enum ModelSelection {}
        );
        let mut violations = Vec::new();

        lint_file_subject(Path::new("src/agent/request.rs"), &file, &mut violations);

        assert!(violations.iter().any(|violation| {
            violation.kind == LintKind::FileSubject && violation.message.contains("ModelSelection")
        }));
    }

    #[test]
    fn accepts_strict_file_subject_companions() {
        let file: File = parse_quote!(
            pub struct AgentRuntimeError;
            pub trait AgentRuntime {}
            pub struct SystemAgentRuntime;
        );
        let mut violations = Vec::new();

        lint_file_subject(Path::new("src/agent/runtime.rs"), &file, &mut violations);

        assert!(violations.is_empty());
    }
}
