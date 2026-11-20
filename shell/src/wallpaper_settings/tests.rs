use super::gallery::{parse_gallery, parse_live_scenes, run_child};
use std::{path::Path, time::Duration};

#[tokio::test]
async fn wallpaper_settings_behaviour() {
    // Wallpaper settings behaviour.
    {
        // Gallery rejects duplicate ids and foreign cache paths.
        {
            let cache = Path::new("/owned/thumbs");
            let entry = serde_json::json!({"id":"a".repeat(64),"digest":"b".repeat(64),"source":"/owned/image.jpg","thumbnail":format!("/owned/thumbs/{}.png","b".repeat(64)),"title":"Image"});
            let packet = |entries| {
                serde_json::to_vec(
                    &serde_json::json!({"entries":entries,"truncated":false,"skipped":0}),
                )
                .unwrap()
            };
            assert!(parse_gallery(&packet(vec![entry.clone()]), cache).is_ok());
            assert!(parse_gallery(&packet(vec![entry.clone(), entry.clone()]), cache).is_err());
            let mut bad = entry;
            bad["thumbnail"] = "/elsewhere/a.png".into();
            assert!(parse_gallery(&packet(vec![bad]), cache).is_err());
        }
        // Live scene lists are bounded and previews stay inside their scene.
        {
            let scenes = Path::new("/opt/enzui/enzwall/scenes");
            let good = br#"[{"id":"aurora","name":"Aurora","description":"x","animated":true,"parallax":true,"preview":"/opt/enzui/enzwall/scenes/aurora/preview.png"},{"id":"still","name":"Still","preview":null}]"#;
            let list = parse_live_scenes(good, scenes).unwrap();
            assert_eq!(list.len(), 2);
            assert!(list[0].animated && !list[1].animated);
            for bad in [
                br#"[{"id":"aurora","name":"A","preview":"/tmp/aurora/preview.png"}]"#.as_slice(),
                br#"[{"id":"Aurora","name":"A","preview":null}]"#.as_slice(),
                br#"[{"id":"a","name":"A","preview":null},{"id":"a","name":"B","preview":null}]"#
                    .as_slice(),
                br#"[{"id":"a","name":"A\nB","preview":null}]"#.as_slice(),
                b"not json".as_slice(),
            ] {
                assert!(parse_live_scenes(bad, scenes).is_err());
            }
        }
    }

    let (sender, receiver) = tokio::sync::watch::channel(false);
    sender.send(true).unwrap();
    let mut command = tokio::process::Command::new("/usr/bin/python3");
    command.args(["-c", "import time; time.sleep(5)"]);
    assert_eq!(
        run_child(command, vec![], receiver, Duration::from_secs(3), 8192)
            .await
            .unwrap_err(),
        "Image operation canceled"
    );
}
