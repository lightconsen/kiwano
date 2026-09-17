//! The walk is tested against injected environments rather than the machine:
//! a temp directory for `home` and a closure for the environment variables, so
//! "it finds a tool" is a fact about the code and not about the developer's
//! laptop. (The shell probe and the platform decoders are tested on their own
//! terms below.)

use super::*;

/// A walk that sees only `home` and no environment variables.
fn bare_env(home: &Path) -> SearchEnv {
    SearchEnv {
        home: home.to_path_buf(),
        path: OsString::new(),
        var: Box::new(|_| None),
        manual: Vec::new(),
    }
}

fn write_bin(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, b"#!/bin/sh\n").unwrap();
    path
}

// ── the well-known-directory walk ──

#[test]
fn finds_a_binary_in_a_well_known_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let expected = write_bin(&home.join(".local/bin"), "kimi");

    assert_eq!(
        search_binary_in("kimi", &bare_env(home)).as_deref(),
        Some(expected.as_path())
    );
}

#[test]
fn a_tool_that_is_nowhere_is_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(search_binary_in("kimi", &bare_env(tmp.path())).is_none());
}

/// Priority, not just presence: OpenCode's own installer directory is listed
/// before the shared `~/.local/bin`, so a native install wins.
#[test]
fn an_installers_own_directory_wins_over_a_shared_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let native = write_bin(&home.join(".opencode/bin"), "opencode");
    write_bin(&home.join(".local/bin"), "opencode");

    assert_eq!(
        search_binary_in("opencode", &bare_env(home)).as_deref(),
        Some(native.as_path())
    );
}

/// Version managers keep one directory per version; the walk has to look inside.
#[test]
fn version_manager_children_are_searched() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let expected = write_bin(&home.join(".nvm/versions/node/v22.19.0/bin"), "pi");

    assert_eq!(
        search_binary_in("pi", &bare_env(home)).as_deref(),
        Some(expected.as_path())
    );
}

/// A tool-specific environment override is honored.
#[test]
fn an_env_override_directory_is_searched() {
    let tmp = tempfile::tempdir().unwrap();
    let override_dir = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&override_dir).unwrap();
    let expected = write_bin(&override_dir, "opencode");

    let env = SearchEnv {
        home: tmp.path().to_path_buf(),
        path: OsString::new(),
        var: Box::new(move |name| {
            (name == "OPENCODE_INSTALL_DIR").then(|| override_dir.clone().into_os_string())
        }),
        manual: Vec::new(),
    };
    assert_eq!(
        search_binary_in("opencode", &env).as_deref(),
        Some(expected.as_path())
    );
}

/// The process PATH is the walk's last source, and it is read as one.
#[test]
fn the_path_environment_is_searched_last() {
    let tmp = tempfile::tempdir().unwrap();
    let on_path = tmp.path().join("on-path");
    let expected = write_bin(&on_path, "qwen");

    let env = SearchEnv {
        home: tmp.path().join("empty-home"),
        path: on_path.clone().into_os_string(),
        var: Box::new(|_| None),
        manual: Vec::new(),
    };
    assert_eq!(
        search_binary_in("qwen", &env).as_deref(),
        Some(expected.as_path())
    );
}

/// A PATH entry that is not absolute does not name a place tools live, so the
/// walk does not open it: `.` is wherever the app was launched from, and a
/// literal `~` is expanded by nobody.
#[test]
fn only_absolute_path_entries_are_searched() {
    let tmp = tempfile::tempdir().unwrap();
    let absolute = tmp.path().join("on-path");
    let mut env = bare_env(tmp.path());
    env.path = std::env::join_paths([
        OsString::from("."),
        OsString::from("~/.dotnet/tools"),
        absolute.clone().into_os_string(),
    ])
    .unwrap();

    let dirs = search_paths("gemini", &env);
    assert!(!dirs.contains(&PathBuf::from(".")), "{dirs:?}");
    assert!(
        !dirs.iter().any(|d| d.to_string_lossy().contains('~')),
        "{dirs:?}"
    );
    assert!(dirs.contains(&absolute), "{dirs:?}");
}

// ── a directory the user declared ──

/// The ordering the whole design rests on: a well-known location wins over a
/// declaration, and a declaration wins over PATH. (Nothing else expresses that
/// order — it is one `push_unique` loop's position.)
#[test]
fn a_declaration_sits_between_the_known_locations_and_path() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let known = write_bin(&home.join(".local/bin"), "kimi");
    let declared = write_bin(&home.join("declared"), "kimi");

    let mut env = bare_env(home);
    env.manual = vec![home.join("declared")];
    assert_eq!(
        search_binary_in("kimi", &env).as_deref(),
        Some(known.as_path()),
        "the location the module is sure of beats the user's word"
    );

    // Only the declared directory has it — the case this feature exists for.
    std::fs::remove_file(&known).unwrap();
    assert_eq!(
        search_binary_in("kimi", &env).as_deref(),
        Some(declared.as_path())
    );

    // And it still beats PATH, which is the walk's last resort.
    write_bin(&home.join("on-path"), "kimi");
    env.path = home.join("on-path").into_os_string();
    assert_eq!(
        search_binary_in("kimi", &env).as_deref(),
        Some(declared.as_path()),
        "a declaration is a statement, not a fallback"
    );
}

/// A declaration only speaks for its own agent.
#[test]
fn a_declaration_does_not_answer_for_another_tool() {
    let tmp = tempfile::tempdir().unwrap();
    let declared = write_bin(&tmp.path().join("declared"), "kimi");

    let mut env = bare_env(tmp.path());
    env.manual = vec![declared.parent().unwrap().to_path_buf()];
    assert!(search_binary_in("gemini", &env).is_none());
}

/// The list the dialog shows under "we looked and did not find it" is the
/// walk's own, so it cannot claim a directory the walk never opens — or miss
/// one it does.
#[test]
fn the_directories_the_dialog_shows_are_the_walks_own() {
    let home = Path::new("/tmp/kiwano-search-dirs-home");

    let dirs = agent_search_dirs("gemini", home, &[]);
    // The per-user prefixes the walk leads with, in its order.
    assert!(dirs.contains(&home.join(".local").join("bin")), "{dirs:?}");
    assert!(dirs.contains(&home.join(".volta").join("bin")), "{dirs:?}");

    // A declaration is a place the walk consults, so it belongs in the list.
    let declared = vec![PathBuf::from("/opt/gemini/bin")];
    assert!(
        agent_search_dirs("gemini", home, &declared).contains(&PathBuf::from("/opt/gemini/bin"))
    );

    // An agent with no command line has nowhere to point at, and no list.
    assert!(agent_search_dirs("workbuddy", home, &[]).is_empty());
}

/// The check a declaration has to pass before it is stored: a directory with
/// the right executable that runs. Its three outcomes are the three things
/// that can go wrong.
#[test]
fn verifying_a_declared_directory_checks_the_tool_itself() {
    let tmp = tempfile::tempdir().unwrap();

    // Nothing by that name in the directory.
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let err = verify_manual_dir("gemini", &empty).unwrap_err();
    assert!(err.contains("no `gemini`"), "{err}");

    // Found, but it is not something that runs.
    let broken = tmp.path().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("gemini"), b"not a program").unwrap();
    let err = verify_manual_dir("gemini", &broken).unwrap_err();
    assert!(err.contains("would not run"), "{err}");

    // A directory that does not exist at all reads as empty, not as an error.
    assert!(verify_manual_dir("gemini", &tmp.path().join("nowhere")).is_err());

    // An id with no command-line tool behind it cannot be declared.
    let err = verify_manual_dir("workbuddy", &tmp.path().join("nowhere")).unwrap_err();
    assert!(err.contains("no command-line tool"), "{err}");
}

// ── what the user's shell says about its own environment ──

/// `env` output, cut down to the names asked for.
#[cfg(unix)]
#[test]
fn a_variable_is_read_out_of_the_shells_environment() {
    let out = "PATH=/usr/bin\nQWEN_HOME=/opt/qwen\nSOMETHING_ELSE=x\n";

    let vars = parse_vars(out, &["QWEN_HOME", "KIMI_SHARE_DIR"]);
    assert_eq!(vars.get("QWEN_HOME").map(String::as_str), Some("/opt/qwen"));
    assert!(
        !vars.contains_key("SOMETHING_ELSE"),
        "a name nobody asked for is not kept"
    );
    assert!(
        !vars.contains_key("KIMI_SHARE_DIR"),
        "nor is a variable the shell does not have"
    );
}

/// A value may contain `=`, and a name that appears twice keeps its first
/// answer — the rule the tool probe uses for the same reason: the first is the
/// one the shell's own environment resolved to.
#[cfg(unix)]
#[test]
fn a_variable_splits_at_its_first_equals_sign() {
    let vars = parse_vars(
        "A=1=2\nQWEN_HOME=first\nQWEN_HOME=second\n",
        &["A", "QWEN_HOME"],
    );
    assert_eq!(vars.get("A").map(String::as_str), Some("1=2"));
    assert_eq!(vars.get("QWEN_HOME").map(String::as_str), Some("first"));
}

// ── executable candidates ──

#[cfg(not(windows))]
#[test]
fn the_candidate_is_the_bare_name() {
    assert_eq!(
        executable_candidates("codex", Path::new("/usr/local/bin")),
        vec![PathBuf::from("/usr/local/bin/codex")]
    );
}

/// Windows tries the script shims before the extensionless name, and only
/// reaches for the bare name when neither shim exists — an npm install leaves a
/// POSIX `#!/bin/sh` shim beside its `.cmd`, and running that shim fails.
#[cfg(windows)]
#[test]
fn the_candidates_prefer_script_shims_and_skip_a_shadowed_bare_name() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let candidates = executable_candidates("codex", dir);
    assert_eq!(candidates[0], dir.join("codex.cmd"));
    assert_eq!(candidates[1], dir.join("codex.exe"));
    assert_eq!(
        candidates.len(),
        3,
        "the bare name is still tried when nothing shadows it"
    );

    std::fs::write(dir.join("codex.cmd"), b"@echo off\n").unwrap();
    let candidates = executable_candidates("codex", dir);
    assert_eq!(
        candidates.len(),
        2,
        "with a .cmd present the extensionless POSIX shim must not be a candidate"
    );
}

// ── version output ──

#[test]
fn version_output_takes_first_non_empty_line() {
    assert_eq!(
        parse_version_output("2.1.83 (Claude Code)"),
        Some("2.1.83 (Claude Code)".into())
    );
    assert_eq!(
        parse_version_output("\n  \n0.5.12\nmore"),
        Some("0.5.12".into())
    );
    assert_eq!(parse_version_output(""), None);
}

// ── the login-shell probe (unix) ──

#[cfg(unix)]
#[test]
fn the_probe_script_lists_every_cli_and_ends_with_true() {
    let s = probe_script();
    for (_, cli) in CLI_AGENTS {
        assert!(s.contains(cli), "script misses {cli}");
    }
    assert!(
        s.ends_with("; true"),
        "a missing last tool must not fail the whole probe: {s}"
    );
}

/// The script must succeed even though some — or, on a fresh machine, all — of
/// the tools it looks for are absent. It runs the real generator rather than a
/// copy, because the failure it guards against was in the script's last line.
#[cfg(unix)]
#[test]
fn the_probe_script_succeeds_when_tools_are_missing() {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(probe_script())
        .output()
        .expect("sh runs");
    assert!(
        out.status.success(),
        "a missing tool must not fail the probe — that is how every installed \
         agent reads as absent"
    );
}

/// Absolute, not `starts_with('/')`: a Windows path is absolute too, and the
/// POSIX spelling of that test discarded every hit there.
#[cfg(unix)]
#[test]
fn probe_output_parses_tools_and_ignores_noise() {
    let out = concat!(
        "\u{1f680} welcome back\n",
        "claude /opt/homebrew/bin/claude\n",
        "codex /usr/local/bin/codex\n",
        "grok /Users/x/.local/bin/grok\n",
        "some random rc line\n",
        "pi /usr/bin/pi\n",
        "gemini relative/gemini\n",
    );
    let m = parse_probe_output(out);
    assert_eq!(
        m.get("claude").unwrap(),
        Path::new("/opt/homebrew/bin/claude")
    );
    assert_eq!(
        m.get("grok").unwrap(),
        Path::new("/Users/x/.local/bin/grok")
    );
    assert_eq!(m.get("pi").unwrap(), Path::new("/usr/bin/pi"));
    // A relative path is not a resolved binary.
    assert!(!m.contains_key("gemini"));
    assert_eq!(m.len(), 4);
}

#[cfg(unix)]
#[test]
fn probe_output_keeps_first_hit_on_duplicates() {
    let m = parse_probe_output("codex /a/codex\ncodex /b/codex\n");
    assert_eq!(m.get("codex").unwrap(), Path::new("/a/codex"));
}

#[cfg(unix)]
#[test]
fn the_probe_flag_matches_the_shell() {
    assert_eq!(login_probe_flag("/bin/sh"), "-c");
    assert_eq!(login_probe_flag("/usr/bin/dash"), "-c");
    assert_eq!(login_probe_flag("/opt/homebrew/bin/fish"), "-lc");
    assert_eq!(login_probe_flag("/bin/zsh"), "-lic");
    assert_eq!(login_probe_flag("/usr/bin/env"), "-lic");
}

#[cfg(unix)]
#[test]
fn only_known_shells_are_used() {
    assert!(is_known_shell("/bin/zsh"));
    assert!(is_known_shell("bash"));
    assert!(!is_known_shell("/usr/bin/python3"));
    assert!(!is_known_shell(""));
}

// ── Windows path handling ──

#[cfg(windows)]
#[test]
fn path_segments_are_merged_in_order_without_case_duplicates() {
    let merged = merge_path_segments(&[
        r"C:\Windows;C:\Tools",
        r"C:\Tools;C:\Users\me\bin",
        r"c:\windows;C:\Program Files\nodejs",
    ]);
    assert_eq!(
        merged,
        r"C:\Windows;C:\Tools;C:\Users\me\bin;C:\Program Files\nodejs"
    );
}

#[cfg(windows)]
#[test]
fn env_expansion_leaves_unknown_names_alone() {
    std::env::set_var("KIWANO_DETECT_TEST", r"C:\expanded");
    assert_eq!(
        expand_env_chars(r"a;%KIWANO_DETECT_TEST%;b"),
        r"a;C:\expanded;b"
    );
    assert_eq!(
        expand_env_chars(r"%KIWANO_DETECT_TEST_NOT_SET%;tail"),
        r"%KIWANO_DETECT_TEST_NOT_SET%;tail"
    );
    assert_eq!(expand_env_chars("100%"), "100%");
}

#[cfg(windows)]
#[test]
fn the_verbatim_prefix_is_stripped_for_the_shell() {
    assert_eq!(
        windows_shell_compatible_path(Path::new(r"\\?\C:\Tools\codex.cmd")),
        PathBuf::from(r"C:\Tools\codex.cmd")
    );
    assert_eq!(
        windows_shell_compatible_path(Path::new(r"\\?\UNC\server\share\codex.cmd")),
        PathBuf::from(r"\\server\share\codex.cmd")
    );
    assert_eq!(
        windows_shell_compatible_path(Path::new(r"C:\Tools\codex.cmd")),
        PathBuf::from(r"C:\Tools\codex.cmd")
    );
}

#[cfg(windows)]
#[test]
fn cmd_files_go_through_the_command_interpreter() {
    let script = tool_command(Path::new(r"C:\Tools\codex.cmd"), &["--version"]);
    assert_eq!(script.get_program(), OsStr::new("cmd"));

    let exe = tool_command(Path::new(r"C:\Tools\codex.exe"), &["--version"]);
    assert_eq!(exe.get_program(), OsStr::new(r"C:\Tools\codex.exe"));
}

// ── the GUI-only agents ──

#[cfg(target_os = "macos")]
#[test]
fn claude_desktop_is_marked_by_its_app_support_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    assert!(!claude_desktop_installed(home));

    std::fs::create_dir_all(home.join("Library/Application Support/Claude")).unwrap();
    assert!(claude_desktop_installed(home));
}

#[test]
fn workbuddy_is_marked_by_its_config_root() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let installed_before = workbuddy_installed(home);

    std::fs::create_dir_all(home.join(".workbuddy")).unwrap();
    assert!(workbuddy_installed(home));

    // The app bundle may also be present on this machine, which is why the
    // "before" state is read rather than assumed.
    let _ = installed_before;
}
