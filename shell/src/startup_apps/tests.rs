use super::*;

fn store(dir: &Path) -> Store {
    Store {
        user: dir.join("user"),
        system: vec![dir.join("system")],
        applications: vec![dir.join("applications")],
        desktops: vec!["enzUI".into()],
    }
}

fn entry(name: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=/usr/bin/true --example \"a b\"\nX-Custom=preserved\n[Desktop Action Other]\nName=Other\nExec=/usr/bin/false\n"
    )
}

/// Generated units that start in `desktop`: the generator writes OnlyShowIn
/// and NotShowIn into an ExecCondition, checked at login.
fn starting(units: &[String], desktop: &str) -> usize {
    units
        .iter()
        .filter(|unit| {
            let Some(condition) = unit
                .lines()
                .find_map(|line| line.strip_prefix("ExecCondition="))
            else {
                return true;
            };
            let words: Vec<&str> = condition.split('"').collect();
            let (program, only, not) = (
                words[0].trim(),
                words.get(1).copied().unwrap_or(""),
                words.get(3).copied().unwrap_or(""),
            );
            std::process::Command::new(program)
                .args([only, not])
                .env_clear()
                .env("XDG_CURRENT_DESKTOP", desktop)
                .status()
                .unwrap()
                .success()
        })
        .count()
}

#[test]
fn startup_apps_behaviour() {
    // Real systemd generator obeys saved overrides and next login additions.
    {
        let generator =
            Path::new("/usr/lib/systemd/user-generators/systemd-xdg-autostart-generator");
        assert!(
            generator.is_file(),
            "real systemd XDG generator is required for this qualification"
        );
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        std::fs::create_dir_all(&store.system[0]).unwrap();
        std::fs::create_dir_all(&store.applications[0]).unwrap();
        let marker = dir.path().join("executed");
        let original = entry("Example").replace(
            "/usr/bin/true --example \"a b\"",
            &format!("/usr/bin/touch {}", marker.display()),
        );
        let id = "org.example.App.desktop";
        std::fs::write(store.system[0].join(id), &original).unwrap();
        let generate_for = |desktop: &str| {
            let output = tempfile::tempdir_in(dir.path()).unwrap();
            let normal = output.path().join("normal");
            let early = output.path().join("early");
            let late = output.path().join("late");
            for path in [&normal, &early, &late] {
                std::fs::create_dir(path).unwrap();
            }
            let config = dir.path().join("config");
            std::fs::create_dir_all(&config).unwrap();
            let link = config.join("autostart");
            if !link.exists() {
                std::os::unix::fs::symlink(&store.user, &link).unwrap();
            }
            let system = dir.path().join("system-config");
            std::fs::create_dir_all(&system).unwrap();
            let link = system.join("autostart");
            if !link.exists() {
                std::os::unix::fs::symlink(&store.system[0], &link).unwrap();
            }
            let result = std::process::Command::new(generator)
                .args([&normal, &early, &late])
                .env_clear()
                .env("HOME", dir.path())
                .env("PATH", "/usr/bin:/bin")
                .env("XDG_CONFIG_HOME", config)
                .env("XDG_CONFIG_DIRS", system)
                .env("XDG_CURRENT_DESKTOP", desktop)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let units: Vec<_> = std::fs::read_dir(&late)
                .unwrap()
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "service"))
                .collect();
            let contents: Vec<_> = units
                .iter()
                .map(|path| std::fs::read_to_string(path).unwrap())
                .collect();
            (output, contents)
        };
        let generate = || generate_for("enzUI:Hyprland");
        std::fs::create_dir_all(&store.user).unwrap();
        let (_, units) = generate();
        assert_eq!(units.len(), 1);
        assert!(units[0].contains("/usr/bin/touch"));
        assert!(!marker.exists());
        let app = store.read().apps.remove(0);
        store.set(id, &app.review, false, false).unwrap();
        assert_eq!(starting(&generate().1, "enzUI:Hyprland"), 0);
        // Turned off in enzUI only: another desktop the user logs in to still starts it.
        // Sabotage: write Hidden=true instead of NotShowIn in Store::set.
        assert_eq!(starting(&generate_for("KDE").1, "KDE"), 1);
        assert!(
            !store.read().apps[0].enabled,
            "an app turned off here reads as off"
        );
        assert!(
            store.read().apps[0].reason.is_none(),
            "enzUI's own off switch is not a reason"
        );
        assert_eq!(
            std::fs::read_to_string(store.system[0].join(id)).unwrap(),
            original
        );
        let app = store.read().apps.remove(0);
        store.set(id, &app.review, true, false).unwrap();
        let (_, units) = generate();
        assert_eq!(units.len(), 1);
        let source = units[0]
            .lines()
            .find_map(|line| line.strip_prefix("SourcePath="))
            .unwrap();
        assert_eq!(
            std::fs::canonicalize(source).unwrap(),
            std::fs::canonicalize(store.user.join(id)).unwrap()
        );
        let infrastructure =
            entry("Certificate storage").replace("X-Custom=preserved", "NoDisplay=true");
        std::fs::write(
            store.system[0].join("certificate-storage.desktop"),
            &infrastructure,
        )
        .unwrap();
        let inventory = store.read();
        assert_eq!(inventory.apps.len(), 1);
        assert_eq!(inventory.apps[0].id, id);
        assert_eq!(
            std::fs::read_to_string(store.system[0].join("certificate-storage.desktop")).unwrap(),
            infrastructure
        );
        std::fs::remove_file(store.system[0].join("certificate-storage.desktop")).unwrap();
        let new_id = "org.example.New.desktop";
        std::fs::write(store.applications[0].join(new_id), entry("New app")).unwrap();
        let candidate = store.read().candidates.remove(0);
        store.set(new_id, &candidate.review, true, true).unwrap();
        let (_, units) = generate();
        assert_eq!(units.len(), 2);
        assert!(units.iter().any(|unit| unit.contains(
            "ExecCondition=/usr/lib/systemd/systemd-xdg-autostart-condition \"enzUI\" \"\""
        )));
        for (index, key) in [
            "X-systemd-skip=true",
            "X-GNOME-Autostart-Phase=Initialization",
            "TryExec=/nonexistent/qa-executable",
        ]
        .iter()
        .enumerate()
        {
            std::fs::write(
                store.system[0].join(format!("org.example.Skipped{index}.desktop")),
                entry("Skipped").replace("X-Custom=preserved", key),
            )
            .unwrap();
        }
        let (_, units) = generate();
        assert_eq!(units.len(), 3);
        assert!(units.iter().any(|unit| unit.contains(
            "ExecCondition=/usr/lib/systemd/systemd-xdg-autostart-condition \"\" \"GNOME\""
        )));
        let phase = Entry::parse(&entry("Phase").replace(
            "X-Custom=preserved",
            "X-GNOME-Autostart-Phase=Initialization",
        ))
        .unwrap();
        assert!(phase.reason(&store.desktops).is_none());
        assert!(phase.reason(&["GNOME".into()]).is_some());
        assert!(!marker.exists());
        let condition = Path::new("/usr/lib/systemd/systemd-xdg-autostart-condition");
        assert!(condition.is_file());
        let matches = |desktop: &str, only: &str, not: &str| {
            std::process::Command::new(condition)
                .args([only, not])
                .env_clear()
                .env("XDG_CURRENT_DESKTOP", desktop)
                .status()
                .unwrap()
                .success()
        };
        assert!(matches("enzUI:Hyprland", "enzUI", ""));
        assert!(!matches("GNOME", "enzUI", ""));
        assert!(!matches("enzUI:Hyprland", "", "enzUI"));
        assert!(!marker.exists());
    }
    // Invalid ids duplicate keys and symlink sources fail closed.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        assert!(!valid_id("../App.desktop"));
        assert!(!valid_id(".hidden.desktop"));
        assert!(!valid_id("App\n.desktop"));
        assert!(
            Entry::parse(
                &entry("Example").replace("X-Custom=preserved", "Hidden=true\nHidden=false")
            )
            .is_err()
        );
        std::fs::create_dir_all(&store.system[0]).unwrap();
        let real = dir.path().join("real");
        std::fs::write(&real, entry("Example")).unwrap();
        std::os::unix::fs::symlink(&real, store.system[0].join("App.desktop")).unwrap();
        let inventory = store.read();
        assert!(inventory.apps.is_empty());
        assert!(inventory.error.is_some());
    }
}
