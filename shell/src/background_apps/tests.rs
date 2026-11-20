use super::*;
use serde_json::json;

fn app() -> Value {
    json!({"schema":1,"app_id":"org.enzui.BackgroundQA","title":"Background QA","purpose":"Public test process",
        "running":true,"unit_observation_known":true,"visibility":"windowless","window_count":0,"minimized_window_count":0,
        "work_observation_known":false,"work":[],"activation_policy":{"user":true,"autostart":false},
        "autostart_supported":true,"autostart_state":"disabled","graceful_stop_supported":false,
        "force_stop_supported":true,"identity_verified":true})
}

fn inventory(app: Value) -> Value {
    json!({"schema":1,"generation":4,"apps":[app],"catalogue_observation_known":true,
        "flatpak_background":null,"unavailable_declarations":[],"policy_operation":null})
}

fn stop_review(app: &App) -> Value {
    json!({"plan_sha256":"a".repeat(64),"plan":{"schema":1,"app_id":app.app_id,"mode":"force",
        "unit":"enzui-background-org.enzui.BackgroundQA.service","main_pid":123,"start_ticks":55,"manifest_sha256":"b".repeat(64),"observation_sha256":"c".repeat(64),
        "may_interrupt_work":true,"may_close_windows":false,"future_activation_policy_changed":false,"personal_files_removed":false}})
}

#[test]
fn background_apps_behaviour() {
    // Ambiguous identity bad schema and unbounded inventory are refused.
    {
        let mut value = inventory(app());
        value["schema"] = 2.into();
        assert!(Snapshot::decode(&value.to_string()).is_err());
        value["schema"] = 1.into();
        value["apps"] = json!([app(), app()]);
        assert!(Snapshot::decode(&value.to_string()).is_err());
        value["apps"] = json!([app()]);
        value["apps"][0]["identity_verified"] = false.into();
        assert!(Snapshot::decode(&value.to_string()).is_err());
        value["apps"][0]["force_stop_supported"] = false.into();
        assert!(!Snapshot::decode(&value.to_string()).unwrap().apps[0].can_stop(StopMode::Force));
        assert!(Snapshot::decode(&" ".repeat(LIMIT + 1)).is_err());
        let mut infrastructure = app();
        infrastructure["app_id"] = "org.enzui.Shell".into();
        assert!(Snapshot::decode(&inventory(infrastructure).to_string()).is_err());
    }
    // Reviewed stop is bound to app mode process and non policy consequences.
    {
        let app = Snapshot::decode(&inventory(app()).to_string())
            .unwrap()
            .apps
            .remove(0);
        let action = Action::Stop {
            app: app.clone(),
            mode: StopMode::Force,
        };
        let mut plan = stop_review(&app);
        assert!(Review::decode(&plan.to_string(), action.clone()).is_ok());
        for (field, changed) in [
            ("app_id", json!("org.example.Other")),
            ("mode", json!("finish-then-exit")),
            ("main_pid", json!(0)),
            ("future_activation_policy_changed", json!(true)),
            ("personal_files_removed", json!(true)),
        ] {
            let mut bad = plan.clone();
            bad["plan"][field] = changed;
            assert!(
                Review::decode(&bad.to_string(), action.clone()).is_err(),
                "{field}"
            );
        }
        plan["plan_sha256"] = "bad".into();
        assert!(Review::decode(&plan.to_string(), action).is_err());
    }
    // Graceful review requires revision bound consequences of the same process.
    {
        let app = Snapshot::decode(&inventory(app()).to_string())
            .unwrap()
            .apps
            .remove(0);
        let action = Action::Stop {
            app: app.clone(),
            mode: StopMode::Finish,
        };
        let mut value = stop_review(&app);
        value["plan"]["mode"] = "finish-then-exit".into();
        value["plan"]["choice"] = "finish-then-exit".into();
        assert!(Review::decode(&value.to_string(), action.clone()).is_err());
        value["plan"]["lifecycle"] = json!({"revision":"revision-2","owner":":1.23","main_pid":123,"start_ticks":55,
            "consequences":[{"kind":"copy","title":"Copy in progress","detail":"Finishing pending work"}]});
        assert!(Review::decode(&value.to_string(), action.clone()).is_ok());
        value["plan"]["lifecycle"]["main_pid"] = 999.into();
        assert!(Review::decode(&value.to_string(), action).is_err());
    }
}
