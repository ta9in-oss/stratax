#!/usr/bin/env python3
"""Generate the synthetic sample data in ui/mock/ (used when the UI runs in a
plain browser, and for screenshots). Everything here is invented: a made-up
user `dev` on host `devbox` with made-up projects. Deterministic, so reruns
produce identical files.

    python3 scripts/gen_mock.py
"""
import hashlib, json, random
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "ui" / "mock"
R = random.Random(7)
NOW = 1_800_000_000          # fixed "scan time" (2027-01-15)
DAY = 86_400
GB, MB = 2**30, 2**20
HOME = "/home/dev"

def fid(rule, path):
    return "f" + hashlib.sha1(f"{rule}\0{path}".encode()).hexdigest()[:12]

LABELS = {
    "agent_session": "Agent sessions", "agent_data": "Agent data", "worktree": "Worktrees",
    "mutation_test": "Mutation tests", "build_artifact": "Build artifacts", "package_cache": "Package caches",
    "toolchain": "Toolchains", "browser_data": "Browsers", "app_cache": "App caches", "container": "Containers",
    "models": "AI models", "trash": "Trash", "system": "System",
}

findings = []

def add(rule, title, path, cat, owner, safety, size, age_days, reason, action=None, protected=None, files=None):
    action = action or {"kind": "delete", "paths": [path]}
    findings.append({
        "id": fid(rule, path), "rule": rule, "title": title, "path": path, "category": cat, "owner": owner,
        "safety": safety, "bytes": int(size), "files": files if files is not None else int(size / (180 * 1024)) + 1,
        "last_activity": None if age_days is None else NOW - int(age_days * DAY), "reason": reason,
        "action": action, "relocate_to": None, "protected": protected, "preselected": False,
    })

PROJECTS = [  # (dir, kind, target GB, idle days, mutation-tested)
    ("atlas-api", "rust", 38.4, 12, True), ("harbor", "rust", 21.7, 2, True), ("quill-engine", "rust", 16.2, 0.02, True),
    ("ledgerline", "rust", 9.6, 41, False), ("tinyfs", "rust", 4.4, 160, False), ("orbit-cli", "rust", 2.9, 6, False),
    ("dash-web", "node", 1.9, 3, False), ("pricing-site", "node", 1.4, 95, False), ("docs-portal", "node", 1.1, 230, False),
    ("mobile-shell", "flutter", 2.6, 58, False), ("ml-notebooks", "python", 6.8, 30, False), ("infra/terraform", "tf", 0.7, 300, False),
]
TARGET_WHY = "Cargo build output. Mutation tests and parallel agents rebuild here constantly, so incremental/ and stale profile dirs balloon."
for name, kind, gb, idle, mut in PROJECTS:
    p = f"{HOME}/code/{name}"
    short = f"~/code/{name}"
    if kind == "rust":
        inc = gb * R.uniform(0.18, 0.32)
        add("rust.target", f"Rust target/ · {short}", f"{p}/target", "build_artifact", "rust", "rebuild", gb * GB, idle,
            f"{TARGET_WHY} incremental/ alone is {inc:.1f} GB.",
            protected="in use by cargo (pid 48213)" if name == "quill-engine" else None)
        if inc > 1.5:
            add("rust.incremental", f"Rust incremental cache · {short}", f"{p}/target/*/incremental", "build_artifact", "rust", "safe",
                inc * GB, idle, "Incremental-compilation cache only: deleting it never breaks a build, the next one is just slower.",
                action={"kind": "delete", "paths": [f"{p}/target/debug/incremental"]},
                protected="in use by cargo (pid 48213)" if name == "quill-engine" else None)
        if mut:
            add("mutants.out", f"cargo-mutants report · {short}", f"{p}/mutants.out", "mutation_test", "cargo-mutants", "safe",
                R.uniform(4, 20) * MB, idle, "Logs + outcomes of the last mutation run.")
            add("mutants.out-old", f"cargo-mutants previous report · {short}", f"{p}/mutants.out.old", "mutation_test", "cargo-mutants", "safe",
                R.uniform(4, 20) * MB, idle + 3, "Previous run, kept by rotation.")
    elif kind == "node":
        add("node.modules", f"node_modules · {short}", f"{p}/node_modules", "build_artifact", "node", "rebuild", gb * GB, idle,
            "Installed dependencies; `pnpm i` / `npm ci` restores them.")
        add("next.cache", f"Next.js build · {short}", f"{p}/.next", "build_artifact", "node", "safe", gb * 0.3 * GB, idle, "Next.js build + cache.")
    elif kind == "flutter":
        add("flutter.build", f"Flutter build/ · {short}", f"{p}/build", "build_artifact", "flutter", "rebuild", gb * GB, idle, "Build output.")
    elif kind == "python":
        add("python.venv", f"Python venv · {short}", f"{p}/.venv", "build_artifact", "python", "rebuild", gb * GB, idle, "Virtualenv; recreate with `uv sync`.")
    else:
        add("terraform", f"Terraform providers · {short}", f"{p}/.terraform", "build_artifact", "terraform", "rebuild", gb * GB, idle, "`terraform init` restores.")

# An orphaned mutation-test copy from a killed run, and a live one.
add("mutants.tmp", "cargo-mutants temp · cargo-mutants-atlas-api-x7Qk.tmp", "/tmp/cargo-mutants-atlas-api-x7Qk.tmp", "mutation_test",
    "cargo-mutants", "safe", 7.3 * GB, 4,
    "cargo-mutants copies the whole crate (and builds a full target/) per job into $TMPDIR. Killed runs leave them behind.")
add("mutants.tmp", "cargo-mutants temp · cargo-mutants-harbor-p2Lm.tmp", "/tmp/cargo-mutants-harbor-p2Lm.tmp", "mutation_test",
    "cargo-mutants", "safe", 2.1 * GB, 0.01, "cargo-mutants copies the whole crate per job into $TMPDIR.",
    protected="a cargo-mutants run is in progress")

# Agent leftovers.
for proj, sid, gb, age, prot in [("atlas-api", "3b9e1c42", 6.8, 9, None), ("harbor", "a71d04ee", 2.4, 0.03, "Claude Code is running in this project and the session is recent"),
                                 ("docs-portal", "5c2f8a19", 1.2, 21, None)]:
    add("claude.tmp-sessions", f"Claude session {sid} · {proj}", f"/tmp/claude-1000/-home-dev-code-{proj}/{sid}-0000-4000-8000-000000000000",
        "agent_session", "claude-code", "safe", gb * GB, age,
        "Per-session scratchpads and background-task output. Agents download, build and log here; nothing cleans it once a session ends.", protected=prot)
add("claude.projects", "Claude Code transcripts", f"{HOME}/.claude/projects", "agent_data", "claude-code", "review", 1.4 * GB, 0.01,
    "Conversation transcripts used by --resume and memory. Old ones can go, but they are history, not cache.", action={"kind": "review_only"})
add("claude.file-history", "Claude Code file history", f"{HOME}/.claude/file-history", "agent_data", "claude-code", "rebuild", 0.3 * GB, 1,
    "Pre-edit file snapshots used for rewind. Only matters for recent sessions.")
add("claude-desktop.vm", "Claude desktop VM bundles", f"{HOME}/.config/Claude/vm_bundles", "agent_data", "claude-desktop", "rebuild", 11.6 * GB, 26,
    "Sandbox VM images for Claude desktop's computer/code environment. Re-downloaded on demand.")
add("codex.runtimes", "Codex runtimes", f"{HOME}/.cache/codex-runtimes", "agent_data", "codex", "rebuild", 1.8 * GB, 33, "Sandboxed runtimes; re-provisioned.")
add("codex.sessions", "Codex sessions", f"{HOME}/.codex/sessions", "agent_data", "codex", "review", 0.9 * GB, 2, "Codex rollout transcripts.", action={"kind": "review_only"})
add("cursor.workspace", "Cursor workspace storage", f"{HOME}/.config/Cursor/User/workspaceStorage", "agent_data", "cursor", "review", 1.7 * GB, 5,
    "Per-workspace state incl. agent chat history.", action={"kind": "review_only"})
add("agent.worktrees", "Agent worktrees · ~/code/atlas-api", f"{HOME}/code/atlas-api/.claude/worktrees", "worktree", "claude-code", "review", 3.1 * GB, 7,
    "Git worktrees an agent created for parallel tasks.", protected="agent-4f1c: 3 uncommitted/untracked change(s)")

# Package caches, toolchains, browsers, models, containers, system.
tool = lambda prog, *args: {"kind": "run_tool", "program": prog, "args": list(args)}
for rule, title, rel, cat, owner, safety, gb, age, why, action in [
    ("cache.pnpm-store", "pnpm store", ".local/share/pnpm/store", "package_cache", "node", "rebuild", 14.2, 1, "Content-addressed store hardlinked into every node_modules. Prune removes packages no project references.", tool("pnpm", "store", "prune")),
    ("cache.uv", "uv cache", ".cache/uv", "package_cache", "python", "safe", 8.9, 2, "uv package cache.", tool("uv", "cache", "prune")),
    ("cache.pip", "pip cache", ".cache/pip", "package_cache", "python", "safe", 5.1, 18, "Wheel/http cache.", tool("pip", "cache", "purge")),
    ("cache.yarn", "Yarn cache", ".cache/yarn", "package_cache", "node", "safe", 4.6, 64, "Yarn v1 tarball cache.", tool("yarn", "cache", "clean")),
    ("cache.npm", "npm cache", ".npm/_cacache", "package_cache", "node", "safe", 3.2, 3, "npm tarball cache.", tool("npm", "cache", "clean", "--force")),
    ("cache.go-build", "Go build cache", ".cache/go-build", "package_cache", "go", "safe", 2.7, 9, "Compiler output cache.", tool("go", "clean", "-cache")),
    ("cache.cargo-registry", "Cargo registry", ".cargo/registry", "package_cache", "rust", "rebuild", 3.6, 0.5, "Downloaded crates (cache/ + extracted src/).", {"kind": "review_only"}),
    ("cache.playwright", "Playwright browsers · chromium-1187", ".cache/ms-playwright/chromium-1187", "package_cache", "node", "rebuild", 0.6, 120, "One browser build per Playwright version.", None),
    ("tool.rustup", "Rust toolchain 1.82.0-x86_64-unknown-linux-gnu", ".rustup/toolchains/1.82.0-x86_64-unknown-linux-gnu", "toolchain", "rust", "rebuild", 1.4, 90, "Non-default toolchain. `rustup toolchain uninstall` it if no project pins it.", tool("rustup", "toolchain", "uninstall", "1.82.0-x86_64-unknown-linux-gnu")),
    ("tool.nvm", "Node v20.18.0", ".nvm/versions/node/v20.18.0", "toolchain", "node", "rebuild", 0.2, 140, "Not the active nvm version.", None),
    ("browser.chrome-cache", "Chrome cache", ".cache/google-chrome", "browser_data", "chrome", "safe", 6.4, 0.01, "HTTP/media cache.", None),
    ("browser.chrome-profile", "Chrome profiles", ".config/google-chrome", "browser_data", "chrome", "review", 12.8, 0.01, "Profile data (history, extensions, IndexedDB, service workers). Not cache — clean from inside Chrome.", {"kind": "review_only"}),
    ("models.hf", "Hugging Face cache", ".cache/huggingface", "models", "ml", "review", 9.3, 44, "Model weights and datasets.", {"kind": "review_only"}),
    ("models.whisper", "Whisper models", ".cache/whisper", "models", "ml", "review", 2.9, 70, "Speech model weights.", {"kind": "review_only"}),
    ("vscode.workspace", "VS Code workspace storage", ".config/Code/User/workspaceStorage", "agent_data", "vscode", "review", 1.3, 1, "Per-workspace state incl. Copilot / Cline / Roo chat history.", {"kind": "review_only"}),
    ("trash", "Trash", ".local/share/Trash", "trash", None, "safe", 3.4, 15, "Already-deleted files still on disk.", None),
]:
    path = f"{HOME}/{rel}"
    prot = "in use by chrome (pid 3120)" if rule.startswith("browser.chrome") else None
    add(rule, title, path, cat, owner, safety, gb * GB, age, why, action, protected=prot)
for i, name in enumerate(["embeddings-cache", "jetbrains-index", "electron-builder"]):
    gb = [4.1, 1.6, 0.9][i]
    add("cache.generic", f"~/.cache/{name}", f"{HOME}/.cache/{name}", "app_cache", None, "rebuild", gb * GB, 20 + i * 30,
        "XDG cache directory that no specific rule knows. Apps must tolerate it disappearing, but some keep expensive-to-rebuild data here (embeddings, indexes) — check before deleting large ones.")
for ty, rule, args, safety, gb, total, why in [
    ("Images", "docker.images", ["image", "prune", "-a", "-f"], "rebuild", 14.6, 31.0, "Images not used by any container; pulled again on demand."),
    ("Build Cache", "docker.build-cache", ["builder", "prune", "-a", "-f"], "safe", 11.9, 18.4, "BuildKit layer cache; agents running `docker build` grow this fast."),
    ("Local Volumes", "docker.volumes", ["volume", "prune", "-f"], "review", 9.2, 12.5, "Volumes no container references — may hold databases from old dev stacks. The action prunes anonymous volumes only; `docker volume prune -a` also removes named ones, check them first."),
    ("Containers", "docker.containers", ["container", "prune", "-f"], "rebuild", 2.3, 2.3, "Stopped containers and their writable layers."),
]:
    path = f"docker://{ty.lower().replace(' ', '-')}"
    add(rule, f"Docker {ty.lower()} ({total} GB total)", path, "container", "docker", safety, gb * 1e9, None, why, tool("docker", *args), files=R.randint(8, 400))
for snap, rev in [("firefox", 5437), ("gnome-46-2404", 145), ("core22", 1663)]:
    add("snap.disabled", f"Old snap {snap} r{rev}", f"/var/lib/snapd/snaps/{snap}_{rev}.snap", "system", "snap", "safe", R.uniform(0.1, 0.6) * GB, None,
        "Disabled snap revision kept for rollback.", tool("sudo", "snap", "remove", snap, f"--revision={rev}"), files=1)
add("apt.archives", "APT package downloads", "/var/cache/apt/archives", "system", "apt", "safe", 0.8 * GB, 6, "Downloaded .deb files.",
    tool("sudo", "apt-get", "clean"))

# Preselection (same rule as the scanner: download caches; safe ≥2 days; rebuild ≥3 weeks).
for f in findings:
    age = (NOW - f["last_activity"]) / DAY if f["last_activity"] else 1e9
    f["preselected"] = (
        f["protected"] is None and f["action"]["kind"] != "review_only" and f["category"] != "toolchain"
        and not (f["rule"] == "cache.generic" and f["bytes"] > GB)
        and ((f["safety"] == "safe" and (f["category"] == "package_cache" or age >= 2)) or (f["safety"] == "rebuild" and age >= 21))
    )
findings.sort(key=lambda f: -f["bytes"])

def totals():
    out = {}
    paths = sorted(findings, key=lambda f: f["path"])
    top = []
    for f in paths:
        if top and not f["path"].startswith("docker://") and f["path"].startswith(top[-1]["path"] + "/"):
            continue
        top.append(f)
    for f in top:
        t = out.setdefault(f["category"], {"category": f["category"], "label": LABELS[f["category"]], "bytes": 0, "reclaimable": 0, "count": 0})
        t["bytes"] += f["bytes"]; t["count"] += 1
        if f["protected"] is None and f["safety"] != "review":
            t["reclaimable"] += f["bytes"]
    return sorted(out.values(), key=lambda t: -t["bytes"])

root_total, root_avail = 980 * GB, 104 * GB
disks = [
    {"name": "nvme0n1p2", "device": "/dev/nvme0n1p2", "label": "system", "fs": "ext4", "mount": "/", "total": root_total,
     "used": root_total - root_avail - 9 * GB, "available": root_avail, "rotational": False, "removable": False,
     "uuid": "00000000-0000-4000-8000-000000000001", "in_fstab": True},
    {"name": "nvme0n1p1", "device": "/dev/nvme0n1p1", "label": "EFI", "fs": "vfat", "mount": "/boot/efi", "total": 1 * GB,
     "used": 62 * MB, "available": 962 * MB, "rotational": False, "removable": False, "uuid": "0000-0001", "in_fstab": True},
    {"name": "sda1", "device": "/dev/sda1", "label": "archive", "fs": "ext4", "mount": None, "total": 1863 * GB,
     "used": 0, "available": 0, "rotational": True, "removable": False, "uuid": "00000000-0000-4000-8000-000000000002", "in_fstab": False},
]
used_root = disks[0]["used"]

agents = [
    {"agent": "claude-code", "kind": "agent", "processes": 3, "threads": 66, "cpu": 14.2, "memory": int(1.1 * GB),
     "cwds": [f"{HOME}/code/harbor", f"{HOME}/code/quill-engine", f"{HOME}/code/dash-web"], "footprint": 0},
    {"agent": "cargo-mutants", "kind": "workload", "processes": 1, "threads": 4, "cpu": 2.1, "memory": 18 * MB,
     "cwds": [f"{HOME}/code/harbor"], "footprint": 0},
    {"agent": "rust-build", "kind": "workload", "processes": 6, "threads": 48, "cpu": 512.0, "memory": int(3.4 * GB),
     "cwds": ["/tmp/cargo-mutants-harbor-p2Lm.tmp", f"{HOME}/code/quill-engine"], "footprint": 0},
    {"agent": "codex", "kind": "agent", "processes": 1, "threads": 18, "cpu": 0.4, "memory": 260 * MB, "cwds": [f"{HOME}/code/atlas-api"], "footprint": 0},
    {"agent": "docker", "kind": "infra", "processes": 2, "threads": 31, "cpu": 1.2, "memory": 88 * MB, "cwds": [], "footprint": 0},
]
for a in agents:
    a["footprint"] = sum(f["bytes"] for f in findings if f["owner"] == a["agent"])

rust = [f for f in findings if f["rule"] == "rust.target"]
mut_projects = {f["path"].rsplit("/", 1)[0] for f in findings if f["category"] == "mutation_test"}
mut_bytes = sum(f["bytes"] for f in rust if f["path"].rsplit("/", 1)[0] in mut_projects)
docker = sum(f["bytes"] for f in findings if f["category"] == "container")
gbs = lambda b: f"{b / GB:.1f} GB" if b < 100 * GB else f"{b / GB:.0f} GB"
growth = {"since": NOW - 3 * DAY, "disks": [{"mount": "/", "delta": int(23 * GB), "per_day": 7.6 * GB, "days_to_full": root_avail / (7.6 * GB)},
                                            {"mount": "/boot/efi", "delta": 0, "per_day": 0.0, "days_to_full": None}],
          "movers": [{"path": "/tmp/cargo-mutants-atlas-api-x7Qk.tmp", "delta": int(7.3 * GB), "now": int(7.3 * GB)},
                     {"path": f"{HOME}/code/harbor/target", "delta": int(5.2 * GB), "now": int(21.7 * GB)}]}
insights = [
    {"severity": "warn", "title": "/ is 90% full", "detail": f"{gbs(root_avail)} free of {gbs(root_total)} on /dev/nvme0n1p2 (NVMe SSD). At the current rate it is full in {root_avail / (7.6 * GB):.1f} days."},
    {"severity": "warn", "title": "HDD archive is not mounted", "detail": "/dev/sda1 (1.8 TB, ext4) is sitting idle. Mount it and move bulky, rarely-read data there — Docker's data-root, model weights, screencasts, old projects — instead of deleting it and watching it come back."},
    {"severity": "warn", "title": f"Rust build output: {gbs(sum(f['bytes'] for f in rust))} in {len(rust)} target/ dirs",
     "detail": f"`cargo clean` in idle projects, or run mutation tests with CARGO_INCREMENTAL=0 and a dedicated CARGO_TARGET_DIR you can wipe after each run. {gbs(mut_bytes)} of that sits in projects that run mutation tests — every mutant triggers a rebuild, so incremental/ grows without bound."},
    {"severity": "warn", "title": f"Docker can give back {gbs(docker)}", "detail": "Build cache and stopped containers come from agents running `docker build` / compose. `docker builder prune -a` is always safe."},
    {"severity": "info", "title": "2 agent process group(s) running: claude-code, codex", "detail": "Anything they hold open or use as a working directory is protected automatically."},
    {"severity": "info", "title": "Fastest grower: +7.3 GB", "detail": "/tmp/cargo-mutants-atlas-api-x7Qk.tmp since the previous scan."},
]
report = {"generated_at": NOW, "host": "devbox", "duration_ms": 38_412, "disks": disks, "findings": findings, "totals": totals(),
          "agents": agents, "growth": growth, "insights": insights, "scan_status": "complete", "unreadable": 0}

# Live resources.
procs = []
def proc(pid, name, cmd, cwd, cpu, mem_mb, thr, agent=None):
    procs.append({"pid": pid, "ppid": 1, "name": name, "cmd": cmd, "cwd": cwd, "cpu": cpu, "memory": int(mem_mb * MB), "threads": thr,
                  "status": "Run" if cpu > 5 else "Sleep", "agent": agent, "run_secs": R.randint(30, 90000), "disk_read": R.randint(0, 9) * MB, "disk_write": R.randint(0, 40) * MB})
for i in range(6):
    proc(48210 + i, "rustc", f"rustc --crate-name crate_{i} --edition=2021 src/lib.rs", "/tmp/cargo-mutants-harbor-p2Lm.tmp", R.uniform(60, 99), R.uniform(300, 900), 8, "rust-build")
proc(47110, "cargo-mutants", "cargo mutants --jobs 4", f"{HOME}/code/harbor", 2.1, 18, 4, "cargo-mutants")
for i, p in enumerate(["harbor", "quill-engine", "dash-web"]):
    proc(31000 + i, "claude", "claude --resume", f"{HOME}/code/{p}", R.uniform(0.5, 9), R.uniform(300, 420), 22, "claude-code")
proc(33100, "codex", "codex", f"{HOME}/code/atlas-api", 0.4, 260, 18, "codex")
proc(2950, "dockerd", "/usr/bin/dockerd -H fd://", "/", 1.0, 70, 22, "docker")
proc(3120, "chrome", "/opt/google/chrome/chrome", HOME, 12.4, 640, 31, "browser")
proc(1420, "gnome-shell", "/usr/bin/gnome-shell", HOME, 6.1, 410, 19)
proc(2210, "postgres", "postgres: 16/main", "/var/lib/postgresql/16/main", 0.6, 120, 1)
proc(2390, "node", "node node_modules/.bin/vite", f"{HOME}/code/dash-web", 3.4, 380, 11, "node-tooling")
for i in range(18):
    proc(5000 + i * 7, R.choice(["bash", "zsh", "systemd", "pipewire", "Xwayland", "sshd", "tmux"]), "", HOME, R.uniform(0, 1.5), R.uniform(4, 60), R.randint(1, 6))
procs.sort(key=lambda p: -p["cpu"])
cores = [R.uniform(55, 98) for _ in range(12)]
groups = {}
for p in procs:
    if not p["agent"]:
        continue
    g = groups.setdefault(p["agent"], {"agent": p["agent"], "kind": {"claude-code": "agent", "codex": "agent", "docker": "infra", "browser": "infra"}.get(p["agent"], "workload"),
                                       "processes": 0, "threads": 0, "cpu": 0.0, "memory": 0, "cwds": [], "footprint": 0})
    g["processes"] += 1; g["threads"] += p["threads"]; g["cpu"] += p["cpu"]; g["memory"] += p["memory"]
    if p["cwd"] not in g["cwds"] and p["cwd"] != "/":
        g["cwds"].append(p["cwd"])
resources = {"at": NOW, "cpu_total": sum(cores) / len(cores), "cpu_cores": cores, "cpu_brand": "Generic 12-core CPU @ 3.2GHz",
             "load": [9.8, 8.1, 6.4], "mem_total": 32 * GB, "mem_used": int(21.4 * GB), "mem_available": int(10.6 * GB),
             "swap_total": 8 * GB, "swap_used": int(1.2 * GB), "uptime": 3 * DAY + 4000, "process_count": len(procs),
             "thread_count": sum(p["threads"] for p in procs), "zombies": 0, "processes": procs,
             "agents": sorted(groups.values(), key=lambda g: -g["memory"])}

# Offline-style advice referencing real mock ids.
by_rule = lambda r, n=0: [f for f in findings if f["rule"] == r][n]
pick = [by_rule("rust.target", 0), by_rule("docker.build-cache"), by_rule("mutants.tmp"), by_rule("claude.tmp-sessions"),
        by_rule("claude-desktop.vm"), by_rule("cache.uv"), by_rule("docker.volumes"), by_rule("models.hf")]
verdicts = ["reclaim", "reclaim", "reclaim", "reclaim", "reclaim", "reclaim", "review", "relocate"]
why = [
    "Largest single item, idle for 12 days and no agent works there. `cargo clean` frees it; the cost is one rebuild.",
    "BuildKit cache from agents' `docker build` runs. `docker builder prune -af` has no data risk.",
    "Left behind by a killed cargo-mutants run; nothing references it.",
    "Scratchpad of a Claude Code session that ended 9 days ago.",
    "Re-downloaded on demand. Quit Claude desktop first.",
    "Pure package cache; `uv cache prune` keeps what installed venvs still use.",
    "May hold databases from old compose stacks. List them with `docker volume ls -f dangling=true` before pruning.",
    "Model weights you probably want to keep: move them to the archive HDD once it is mounted.",
]
advice = {"provider": "offline rules", "summary": "/ is 90% full and filling at about 7.6 GB a day, mostly from Rust builds that mutation tests keep re-running.",
          "recommendations": [{"id": f["id"], "priority": 1 if i < 3 else 2 if i < 6 else 3, "verdict": verdicts[i], "rationale": why[i]} for i, f in enumerate(pick)],
          "prevention": [
              "Run mutation tests with a throwaway target dir: `CARGO_TARGET_DIR=/tmp/mutants-target CARGO_INCREMENTAL=0 cargo mutants`, then delete it.",
              "Wipe stale build output weekly: `stratax clean --preselected` from cron or a systemd timer.",
              "Cap Docker's build cache with `builder.gc` and `defaultKeepStorage: \"20GB\"` in /etc/docker/daemon.json.",
              "Mount the archive HDD at boot (add it to /etc/fstab) and move model caches and old projects onto it.",
          ]}

# 30 days of history: steady growth with two cleanups.
history, used = [], used_root - 120 * GB
for d in range(30, -1, -1):
    for _ in range(R.choice([0, 1, 1, 2])):
        used += R.uniform(2, 9) * GB
        if d in (19, 8):
            used -= R.uniform(60, 90) * GB
        history.append({"at": NOW - d * DAY + R.randint(0, 40000), "disks": {"/": int(min(used, used_root)), "/boot/efi": 62 * MB}})
history.append({"at": NOW, "disks": {"/": used_root, "/boot/efi": 62 * MB}})
history.sort(key=lambda h: h["at"])

OUT.mkdir(parents=True, exist_ok=True)
for name, data in [("report", report), ("resources", resources), ("advice", advice), ("history", history)]:
    (OUT / f"{name}.json").write_text(json.dumps(data, indent=1, ensure_ascii=False) + "\n")
print(f"wrote {len(findings)} findings, {len(procs)} processes, {len(history)} history points to {OUT}")
