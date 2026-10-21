//! The knowledge base: where coding agents, mutation testers and toolchains
//! leave data behind, and how risky it is to reclaim each kind.

use crate::model::{Action, Category, Safety};
use std::path::{Path, PathBuf};

/// A fixed location (relative to `$HOME`, or absolute after `{tmp}` / `/`).
pub struct Known {
    pub id: &'static str,
    pub path: &'static str,
    pub title: &'static str,
    pub category: Category,
    pub owner: Option<&'static str>,
    pub safety: Safety,
    pub reason: &'static str,
    pub how: How,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum How {
    Delete,
    /// Delete the *contents* (keep the directory, tools expect it).
    Empty,
    Review,
    Tool(&'static str, &'static [&'static str]),
    /// One finding per child entry (sessions, worktrees, toolchains...).
    PerChild,
}

macro_rules! k {
    ($id:expr, $path:expr, $title:expr, $cat:ident, $owner:expr, $safety:ident, $how:expr, $reason:expr) => {
        Known {
            id: $id,
            path: $path,
            title: $title,
            category: Category::$cat,
            owner: $owner,
            safety: Safety::$safety,
            reason: $reason,
            how: $how,
        }
    };
}

pub const KNOWN: &[Known] = &[
    // ── Coding agents ────────────────────────────────────────────────────
    k!("claude.tmp-sessions", "{tmp}/claude-{uid}", "Claude Code session temp", AgentSession, Some("claude-code"), Safe, How::PerChild,
       "Per-session scratchpads and background-task output. Agents download, build and log here; nothing cleans it once a session ends."),
    k!("claude.projects", "~/.claude/projects", "Claude Code transcripts", AgentData, Some("claude-code"), Review, How::Review,
       "Conversation transcripts used by --resume and memory. Old ones can go, but they are history, not cache."),
    k!("claude.file-history", "~/.claude/file-history", "Claude Code file history", AgentData, Some("claude-code"), Rebuild, How::Empty,
       "Pre-edit file snapshots used for rewind. Only matters for recent sessions."),
    k!("claude.shell-snapshots", "~/.claude/shell-snapshots", "Claude Code shell snapshots", AgentData, Some("claude-code"), Safe, How::Empty,
       "Shell environment captures; recreated per session."),
    k!("claude.debug", "~/.claude/debug", "Claude Code debug logs", AgentData, Some("claude-code"), Safe, How::Empty, "Debug logs."),
    k!("claude.plugin-cache", "~/.claude/plugins/cache", "Claude Code plugin cache", AgentData, Some("claude-code"), Rebuild, How::Review,
       "Downloaded plugin versions; old versions pile up after updates."),
    k!("claude.versions", "~/.local/share/claude/versions", "Old Claude Code binaries", Toolchain, Some("claude-code"), Rebuild, How::PerChild,
       "Each auto-update keeps the previous native binary."),
    k!("claude-desktop.vm", "~/.config/Claude/vm_bundles", "Claude desktop VM bundles", AgentData, Some("claude-desktop"), Rebuild, How::Delete,
       "Sandbox VM images for Claude desktop's computer/code environment. Re-downloaded on demand."),
    k!("claude-desktop.cache", "~/.config/Claude/Cache", "Claude desktop cache", AppCache, Some("claude-desktop"), Safe, How::Empty, "Electron HTTP cache."),
    k!("claude-desktop.codecache", "~/.config/Claude/Code Cache", "Claude desktop code cache", AppCache, Some("claude-desktop"), Safe, How::Empty, "V8 code cache."),
    k!("codex.sessions", "~/.codex/sessions", "Codex sessions", AgentData, Some("codex"), Review, How::Review, "Codex rollout transcripts."),
    k!("codex.worktrees", "~/.codex/worktrees", "Codex worktrees", Worktree, Some("codex"), Review, How::PerChild,
       "Git worktrees Codex created for parallel tasks."),
    k!("codex.runtimes", "~/.cache/codex-runtimes", "Codex runtimes", AgentData, Some("codex"), Rebuild, How::Delete, "Sandboxed runtimes; re-provisioned."),
    k!("codex.log", "~/.codex/log", "Codex logs", AgentData, Some("codex"), Safe, How::Empty, "Logs."),
    k!("cursor.worktrees", "~/.cursor/worktrees", "Cursor worktrees", Worktree, Some("cursor"), Review, How::PerChild, "Background-agent worktrees."),
    k!("cursor.cache", "~/.config/Cursor/Cache", "Cursor cache", AppCache, Some("cursor"), Safe, How::Empty, "Electron HTTP cache."),
    k!("cursor.cached-data", "~/.config/Cursor/CachedData", "Cursor cached data", AppCache, Some("cursor"), Safe, How::Empty, "Compiled extension cache."),
    k!("cursor.workspace", "~/.config/Cursor/User/workspaceStorage", "Cursor workspace storage", AgentData, Some("cursor"), Review, How::Review,
       "Per-workspace state incl. agent chat history."),
    k!("vscode.cache", "~/.config/Code/Cache", "VS Code cache", AppCache, Some("vscode"), Safe, How::Empty, "Electron HTTP cache."),
    k!("vscode.cached-data", "~/.config/Code/CachedData", "VS Code cached data", AppCache, Some("vscode"), Safe, How::Empty, "Compiled cache."),
    k!("vscode.vsix", "~/.config/Code/CachedExtensionVSIXs", "VS Code extension downloads", AppCache, Some("vscode"), Safe, How::Empty, "Downloaded VSIX files."),
    k!("vscode.workspace", "~/.config/Code/User/workspaceStorage", "VS Code workspace storage", AgentData, Some("vscode"), Review, How::Review,
       "Per-workspace state incl. Copilot / Cline / Roo chat history."),
    // ── Mutation testing (orphaned temp copies) ──────────────────────────
    k!("mutants.tmp", "{tmp}", "cargo-mutants temp copies", MutationTest, Some("cargo-mutants"), Safe, How::PerChild,
       "cargo-mutants copies the whole crate (and builds a full target/) per job into $TMPDIR. Killed runs leave them behind."),
    // ── Package caches ───────────────────────────────────────────────────
    k!("cache.pip", "~/.cache/pip", "pip cache", PackageCache, Some("python"), Safe, How::Tool("pip", &["cache", "purge"]), "Wheel/http cache."),
    k!("cache.uv", "~/.cache/uv", "uv cache", PackageCache, Some("python"), Safe, How::Tool("uv", &["cache", "prune"]), "uv package cache."),
    k!("cache.yarn", "~/.cache/yarn", "Yarn cache", PackageCache, Some("node"), Safe, How::Tool("yarn", &["cache", "clean"]), "Yarn v1 tarball cache."),
    k!("cache.npm", "~/.npm/_cacache", "npm cache", PackageCache, Some("node"), Safe, How::Tool("npm", &["cache", "clean", "--force"]), "npm tarball cache."),
    k!("cache.pnpm-meta", "~/.cache/pnpm", "pnpm metadata cache", PackageCache, Some("node"), Safe, How::Empty, "Registry metadata cache."),
    k!("cache.pnpm-store", "~/.local/share/pnpm/store", "pnpm store", PackageCache, Some("node"), Rebuild, How::Tool("pnpm", &["store", "prune"]),
       "Content-addressed store hardlinked into every node_modules. Prune removes packages no project references."),
    k!("cache.bun", "~/.bun/install/cache", "Bun cache", PackageCache, Some("node"), Safe, How::Tool("bun", &["pm", "cache", "rm"]), "Bun package cache."),
    k!("cache.go-build", "~/.cache/go-build", "Go build cache", PackageCache, Some("go"), Safe, How::Tool("go", &["clean", "-cache"]), "Compiler output cache."),
    k!("cache.go-mod", "~/go/pkg/mod", "Go module cache", PackageCache, Some("go"), Rebuild, How::Tool("go", &["clean", "-modcache"]), "Downloaded modules."),
    k!("cache.cargo-registry", "~/.cargo/registry", "Cargo registry", PackageCache, Some("rust"), Rebuild, How::Review,
       "Downloaded crates (cache/ + extracted src/). `cargo cache --autoclean` trims it."),
    k!("cache.cargo-git", "~/.cargo/git", "Cargo git checkouts", PackageCache, Some("rust"), Rebuild, How::Delete, "Git dependencies; re-fetched."),
    k!("cache.cargo-xwin", "~/.cache/cargo-xwin", "cargo-xwin SDK", Toolchain, Some("rust"), Rebuild, How::Delete, "MSVC CRT/SDK download."),
    k!("cache.puppeteer", "~/.cache/puppeteer", "Puppeteer browsers", PackageCache, Some("node"), Rebuild, How::Delete, "Downloaded Chrome builds."),
    k!("cache.playwright", "~/.cache/ms-playwright", "Playwright browsers", PackageCache, Some("node"), Rebuild, How::PerChild, "One browser build per Playwright version."),
    k!("cache.gradle", "~/.gradle/caches", "Gradle caches", PackageCache, Some("jvm"), Rebuild, How::Delete, "Dependency + build cache."),
    k!("cache.maven", "~/.m2/repository", "Maven repository", PackageCache, Some("jvm"), Rebuild, How::Review, "Downloaded artifacts."),
    k!("cache.composer", "~/.cache/composer", "Composer cache", PackageCache, Some("php"), Safe, How::Empty, "Package cache."),
    k!("cache.phpactor", "~/.cache/phpactor", "phpactor index", AppCache, Some("php"), Safe, How::Empty, "Language-server index; rebuilt."),
    k!("cache.node-gyp", "~/.cache/node-gyp", "node-gyp headers", PackageCache, Some("node"), Safe, How::Empty, "Node headers."),
    k!("cache.deno", "~/.cache/deno", "Deno cache", PackageCache, Some("deno"), Safe, How::Empty, "Module cache."),
    k!("cache.electron", "~/.cache/electron", "Electron downloads", PackageCache, Some("node"), Safe, How::Empty, "Prebuilt Electron zips."),
    // ── Models (large, re-downloadable, but deliberate) ──────────────────
    k!("models.hf", "~/.cache/huggingface", "Hugging Face cache", Models, Some("ml"), Review, How::Review, "Model weights and datasets."),
    k!("models.ollama", "~/.ollama/models", "Ollama models", Models, Some("ml"), Review, How::Review, "Local LLM weights (`ollama rm`)."),
    k!("models.whisper", "~/.cache/whisper", "Whisper models", Models, Some("ml"), Review, How::Review, "Speech model weights."),
    k!("models.lmstudio", "~/.lmstudio/models", "LM Studio models", Models, Some("ml"), Review, How::Review, "Local LLM weights."),
    // ── Toolchains ───────────────────────────────────────────────────────
    k!("tool.rustup", "~/.rustup/toolchains", "Rust toolchains", Toolchain, Some("rust"), Rebuild, How::PerChild, "Installed toolchains; only the default is needed."),
    k!("tool.pyenv", "~/.pyenv/versions", "pyenv Pythons", Toolchain, Some("python"), Rebuild, How::PerChild, "Installed interpreters."),
    k!("tool.nvm", "~/.nvm/versions/node", "nvm Node versions", Toolchain, Some("node"), Rebuild, How::PerChild, "Installed Node versions."),
    k!("tool.android-images", "~/Android/Sdk/system-images", "Android system images", Toolchain, Some("android"), Review, How::Review, "Emulator images."),
    // ── Browsers ─────────────────────────────────────────────────────────
    k!("browser.chrome-cache", "~/.cache/google-chrome", "Chrome cache", BrowserData, Some("chrome"), Safe, How::Empty, "HTTP/media cache."),
    k!("browser.chromium-cache", "~/.cache/chromium", "Chromium cache", BrowserData, Some("chromium"), Safe, How::Empty, "HTTP/media cache."),
    k!("browser.chrome-profile", "~/.config/google-chrome", "Chrome profiles", BrowserData, Some("chrome"), Review, How::Review,
       "Profile data (history, extensions, IndexedDB, service workers). Not cache — clean from inside Chrome."),
    k!("browser.mozilla-cache", "~/.cache/mozilla", "Firefox cache", BrowserData, Some("firefox"), Safe, How::Empty, "HTTP cache."),
    // ── Misc ─────────────────────────────────────────────────────────────
    k!("trash", "~/.local/share/Trash", "Trash", Trash, None, Safe, How::Empty, "Already-deleted files still on disk."),
    k!("screencasts", "~/Screencasts", "Screencasts", AppCache, None, Review, How::Review, "Screen recordings."),
];

/// Directories under `~/.cache` already covered by a specific rule.
pub fn covered_cache_children() -> Vec<&'static str> {
    KNOWN
        .iter()
        .filter_map(|k| k.path.strip_prefix("~/.cache/"))
        .map(|p| p.split('/').next().unwrap())
        .collect()
}

/// Project-level artifacts found by walking source roots.
pub struct ArtifactRule {
    pub id: &'static str,
    pub dir: &'static str,
    /// One of these siblings must exist in the parent ("" = no check).
    pub markers: &'static [&'static str],
    pub title: &'static str,
    pub category: Category,
    pub owner: &'static str,
    pub safety: Safety,
    pub reason: &'static str,
}

pub const ARTIFACTS: &[ArtifactRule] = &[
    ArtifactRule { id: "rust.target", dir: "target", markers: &["Cargo.toml", "target/CACHEDIR.TAG"], title: "Rust target/", category: Category::BuildArtifact, owner: "rust", safety: Safety::Rebuild,
        reason: "Cargo build output. Mutation tests and parallel agents rebuild here constantly, so incremental/ and stale profile dirs balloon." },
    ArtifactRule { id: "node.modules", dir: "node_modules", markers: &["package.json"], title: "node_modules", category: Category::BuildArtifact, owner: "node", safety: Safety::Rebuild,
        reason: "Installed dependencies; `pnpm i` / `npm ci` restores them." },
    ArtifactRule { id: "next.cache", dir: ".next", markers: &["package.json"], title: "Next.js build", category: Category::BuildArtifact, owner: "node", safety: Safety::Safe, reason: "Next.js build + cache." },
    ArtifactRule { id: "turbo.cache", dir: ".turbo", markers: &[], title: "Turborepo cache", category: Category::BuildArtifact, owner: "node", safety: Safety::Safe, reason: "Task cache." },
    ArtifactRule { id: "nuxt.cache", dir: ".nuxt", markers: &["package.json"], title: "Nuxt build", category: Category::BuildArtifact, owner: "node", safety: Safety::Safe, reason: "Generated build." },
    ArtifactRule { id: "svelte.cache", dir: ".svelte-kit", markers: &["package.json"], title: "SvelteKit build", category: Category::BuildArtifact, owner: "node", safety: Safety::Safe, reason: "Generated build." },
    ArtifactRule { id: "parcel.cache", dir: ".parcel-cache", markers: &[], title: "Parcel cache", category: Category::BuildArtifact, owner: "node", safety: Safety::Safe, reason: "Bundler cache." },
    ArtifactRule { id: "angular.cache", dir: ".angular", markers: &["angular.json"], title: "Angular cache", category: Category::BuildArtifact, owner: "node", safety: Safety::Safe, reason: "Build cache." },
    ArtifactRule { id: "expo.cache", dir: ".expo", markers: &["package.json"], title: "Expo cache", category: Category::BuildArtifact, owner: "node", safety: Safety::Safe, reason: "Dev cache." },
    ArtifactRule { id: "python.venv", dir: ".venv", markers: &[".venv/pyvenv.cfg"], title: "Python venv", category: Category::BuildArtifact, owner: "python", safety: Safety::Rebuild, reason: "Virtualenv; recreate with `uv sync`." },
    ArtifactRule { id: "python.venv2", dir: "venv", markers: &["venv/pyvenv.cfg"], title: "Python venv", category: Category::BuildArtifact, owner: "python", safety: Safety::Rebuild, reason: "Virtualenv." },
    ArtifactRule { id: "gradle.project", dir: ".gradle", markers: &["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"], title: "Gradle project cache", category: Category::BuildArtifact, owner: "jvm", safety: Safety::Safe, reason: "Project-local Gradle cache." },
    ArtifactRule { id: "gradle.build", dir: "build", markers: &["build.gradle", "build.gradle.kts"], title: "Gradle build/", category: Category::BuildArtifact, owner: "jvm", safety: Safety::Rebuild, reason: "Build output." },
    ArtifactRule { id: "flutter.build", dir: "build", markers: &["pubspec.yaml"], title: "Flutter build/", category: Category::BuildArtifact, owner: "flutter", safety: Safety::Rebuild, reason: "Build output." },
    ArtifactRule { id: "flutter.dart-tool", dir: ".dart_tool", markers: &["pubspec.yaml"], title: ".dart_tool", category: Category::BuildArtifact, owner: "flutter", safety: Safety::Rebuild, reason: "Generated Dart tooling." },
    ArtifactRule { id: "cmake.build", dir: "build", markers: &["CMakeLists.txt"], title: "CMake build/", category: Category::BuildArtifact, owner: "c++", safety: Safety::Rebuild, reason: "Build tree." },
    ArtifactRule { id: "zig.cache", dir: ".zig-cache", markers: &[], title: "Zig cache", category: Category::BuildArtifact, owner: "zig", safety: Safety::Safe, reason: "Compiler cache." },
    ArtifactRule { id: "terraform", dir: ".terraform", markers: &[], title: "Terraform providers", category: Category::BuildArtifact, owner: "terraform", safety: Safety::Rebuild, reason: "`terraform init` restores." },
    ArtifactRule { id: "mutants.out", dir: "mutants.out", markers: &[], title: "cargo-mutants report", category: Category::MutationTest, owner: "cargo-mutants", safety: Safety::Safe, reason: "Logs + outcomes of the last mutation run." },
    ArtifactRule { id: "mutants.out-old", dir: "mutants.out.old", markers: &[], title: "cargo-mutants previous report", category: Category::MutationTest, owner: "cargo-mutants", safety: Safety::Safe, reason: "Previous run, kept by rotation." },
    ArtifactRule { id: "stryker.tmp", dir: ".stryker-tmp", markers: &[], title: "Stryker sandboxes", category: Category::MutationTest, owner: "stryker", safety: Safety::Safe, reason: "Per-run sandbox copies of the project (incl. node_modules links)." },
    ArtifactRule { id: "mutmut.cache", dir: ".mutmut-cache", markers: &[], title: "mutmut cache", category: Category::MutationTest, owner: "mutmut", safety: Safety::Safe, reason: "Mutation-test cache." },
    ArtifactRule { id: "mutmut.mutants", dir: "mutants", markers: &["setup.cfg", "pyproject.toml"], title: "mutmut mutants/", category: Category::MutationTest, owner: "mutmut", safety: Safety::Review, reason: "mutmut 3 copies the project here per run." },
    ArtifactRule { id: "agent.worktrees", dir: "worktrees", markers: &[], title: "Agent worktrees", category: Category::Worktree, owner: "claude-code", safety: Safety::Review, reason: "Git worktrees an agent created for parallel tasks." },
];

/// Hidden directories the project walker still descends into / matches.
pub const WALK_HIDDEN_OK: &[&str] =
    &[".claude", ".next", ".turbo", ".nuxt", ".svelte-kit", ".parcel-cache", ".angular", ".expo", ".venv", ".gradle", ".dart_tool", ".zig-cache", ".terraform", ".stryker-tmp", ".mutmut-cache", ".codex", ".cursor"];

pub fn expand(path: &str, home: &Path, tmp: &Path, uid: u32) -> PathBuf {
    let s = path.replace("{uid}", &uid.to_string());
    if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else if let Some(rest) = s.strip_prefix("{tmp}") {
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() { tmp.to_path_buf() } else { tmp.join(rest) }
    } else {
        PathBuf::from(s)
    }
}

pub fn action_for(how: How, path: &Path) -> Action {
    match how {
        How::Delete | How::PerChild => Action::Delete { paths: vec![path.to_path_buf()] },
        How::Empty => Action::Delete {
            paths: std::fs::read_dir(path).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default(),
        },
        How::Review => Action::ReviewOnly,
        How::Tool(p, a) => Action::RunTool { program: p.into(), args: a.iter().map(|s| s.to_string()).collect() },
    }
}

/// Claude Code names project temp dirs after the cwd with `/` and `.` → `-`.
pub fn claude_slug(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c == '/' || c == '.' || c == '_' { '-' } else { c }).collect()
}
