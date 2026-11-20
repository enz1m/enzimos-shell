//! Exercises plugin sandbox resource limits using disposable workers.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "This executable is a disposable fault-injection fixture; failed assertions terminate the probe and stdout synchronizes its supervisor."
)]
use enzui_shell::i18n;
use enzui_shell::plugins::{
    budget::BudgetPool,
    manifest::{MAX_PROGRAM_BYTES, Package},
    worker::{Fault, Worker},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

fn package(id: &str, body: &str) -> Package {
    let program = format!(
        "import sys,json,os,time\nprint(json.dumps({{\"type\":\"hello\",\"protocol\":1,\"plugin\":{}}}),flush=True)\nstart=json.loads(sys.stdin.readline())\n{body}\n",
        json!(id)
    );
    Package::parse(&serde_json::to_vec(&json!({"format":1,"id":id,"name":"Disposable fixture","version":"1","description":"Owned plugin fault fixture","permissions":[],"elements":[],"settings":[],"program":program})).unwrap()).unwrap()
}
async fn spawn(pool: &BudgetPool, id: &str, body: &str) -> Worker {
    Worker::spawn(pool, &package(id, body), &json!({}), &BTreeSet::new())
        .await
        .unwrap_or_else(|e| panic!("{id}: {e}"))
}
async fn reclaim(pool: &BudgetPool, path: &Path, remaining: usize) {
    for _ in 0..100 {
        pool.reap();
        let count = std::fs::read_dir(path)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("w-"))
            .count();
        if count == remaining {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Owned worker groups were not reclaimed");
}
async fn reject_bootstrap(pool: &BudgetPool, option: &str, bytes: &[u8]) {
    use tokio::io::AsyncWriteExt;
    let slot = pool.slot().unwrap();
    let mut child = tokio::process::Command::new(env!("ENZUI_PLUGIN_WORKER"))
        .arg(slot.path())
        .arg(option)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let _ = tokio::time::timeout(Duration::from_millis(500), input.write_all(bytes)).await;
    drop(input);
    let output = tokio::time::timeout(Duration::from_secs(2), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(126),
        "sandbox violations must terminate the worker with status 126"
    );
    assert!(
        output.stdout.is_empty(),
        "denied workers must not produce protocol output"
    );
}
async fn ping(worker: &mut Worker, number: u64) {
    worker.send(&json!({"ping":number})).await.unwrap();
    assert_eq!(
        worker.receive().await.unwrap(),
        json!({"pong":number}),
        "worker replies must preserve request sequence numbers"
    );
}
async fn abrupt_supervisor(pool: &BudgetPool) -> ! {
    use std::io::{Read, Write};
    let mut worker = spawn(
        pool,
        "abrupt",
        "print(json.dumps({'ready':True}),flush=True)\nwhile True: time.sleep(1)",
    )
    .await;
    assert_eq!(
        worker.receive().await.unwrap(),
        json!({"ready":true}),
        "the worker must finish its startup handshake"
    );
    println!("READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    assert_eq!(
        byte,
        [b'X'],
        "the supervisor must explicitly release this fixture"
    );
    std::process::exit(0)
}
async fn supervisor_death(nonce: &str, root: &Path) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["abrupt-source", nonce])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut output = child.stdout.take().unwrap();
    let mut ready = [0; 6];
    tokio::time::timeout(Duration::from_secs(5), output.read_exact(&mut ready))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        &ready, b"READY\n",
        "the child must signal readiness before its supervisor exits"
    );
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let ticks = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap();
    let prefix = format!("plugins-{pid}-{ticks}-");
    let paths: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .filter(|p| p.file_name().to_string_lossy().starts_with(&prefix))
        .map(|p| p.path())
        .collect();
    assert_eq!(
        paths.len(),
        1,
        "the probe must own exactly one reservation subtree"
    );
    child.stdin.take().unwrap().write_all(b"X").await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success(),
        "the owned child must terminate successfully before the deadline"
    );
    for _ in 0..100 {
        if std::fs::read_to_string(paths[0].join("cgroup.events"))
            .unwrap()
            .lines()
            .any(|l| l == "populated 0")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        std::fs::read_to_string(paths[0].join("cgroup.events"))
            .unwrap()
            .lines()
            .any(|l| l == "populated 0"),
        "the owned cgroup must contain no live processes"
    );
    for child in std::fs::read_dir(&paths[0]).unwrap().flatten() {
        if child.file_name().to_string_lossy().starts_with("w-") {
            std::fs::remove_dir(child.path()).unwrap();
        }
    }
    std::fs::remove_dir(&paths[0]).unwrap();
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    i18n::initialize();
    let mode = std::env::args().nth(1).unwrap();
    assert!(
        matches!(mode.as_str(), "disposable" | "abrupt-source"),
        "only disposable probe modes may run destructive fixtures"
    );
    let membership = std::fs::read_to_string("/proc/self/cgroup").unwrap();
    let nonce = std::env::args().nth(2).expect("explicit probe identity");
    assert!(
        nonce.len() == 16
            && nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "the owned cgroup nonce must be lowercase hexadecimal"
    );
    let suffix = format!("/enzui-plugin-probe-{nonce}.service/supervisor");
    assert!(
        membership
            .lines()
            .any(|v| v.starts_with("0::") && v.ends_with(&suffix)),
        "this process must belong to its recorded owned cgroup"
    );
    let pool = BudgetPool::from_environment().unwrap();
    if mode == "abrupt-source" {
        abrupt_supervisor(&pool).await;
    }
    let home = std::env::var("HOME").expect("probe host home directory");
    let runtime = std::env::var("XDG_RUNTIME_DIR").expect("probe host runtime directory");
    assert!(
        Path::new(&home).is_dir() && Path::new(&runtime).is_dir(),
        "the probe must test paths that are visible to its supervisor"
    );
    let body = format!(
        "assert not os.path.exists({}) and not os.path.exists({})\n\
         print(json.dumps({{'ready':True}}),flush=True)\n\
         for line in sys.stdin:\n message=json.loads(line)\n print(json.dumps({{'pong':message['ping']}}),flush=True)",
        json!(home),
        json!(runtime)
    );
    let mut healthy = spawn(&pool, "healthy", &body).await;
    assert_eq!(
        healthy.receive().await.unwrap(),
        json!({"ready":true}),
        "the worker must finish its startup handshake"
    );
    let slot = pool.slot().unwrap();
    let pool_path = slot.path().parent().unwrap().to_owned();
    drop(slot);
    let mut results = Vec::<Value>::new();
    let mut reservations = Vec::new();
    for _ in 1..enzui_shell::plugins::budget::MAX_WORKERS {
        reservations.push(pool.slot().unwrap());
    }
    assert!(
        pool.slot().is_err(),
        "the reservation pool must reject exhaustion"
    );
    drop(reservations);
    drop(pool.slot().unwrap());
    results.push(json!({"case":"pool-limit","limit":enzui_shell::plugins::budget::MAX_WORKERS}));
    for (name, option, header, size, body) in [
        (
            "bootstrap-magic",
            "--pipe-source-v2",
            b"INVALID!",
            1u32,
            "x",
        ),
        ("bootstrap-empty", "--pipe-source-v2", b"ENZPY002", 0, ""),
        (
            "bootstrap-oversize",
            "--pipe-source-v2",
            b"ENZPY002",
            MAX_PROGRAM_BYTES as u32 + 1,
            "",
        ),
        (
            "bootstrap-truncated",
            "--pipe-source-v2",
            b"ENZPY002",
            100,
            "print('must not execute')",
        ),
        ("bootstrap-abi", "--pipe-source-v1", b"ENZPY002", 1, "x"),
    ] {
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(body.as_bytes());
        reject_bootstrap(&pool, option, &bytes).await;
        reclaim(&pool, &pool_path, 1).await;
        ping(&mut healthy, 0).await;
        results.push(json!({"case":name,"exit":126}));
    }
    let mut readonly=spawn(&pool,"readonly","try:\n open('/plugin/main.py','w')\nexcept OSError:\n pass\nelse:\n raise AssertionError('writable source')\nassert os.listdir('/plugin')==['main.py']\nassert next(line for line in open('/proc/self/mountinfo') if line.split()[4]=='/plugin/main.py').split(' - ')[1].split()[0]=='tmpfs'\nassert os.stat('/plugin/main.py').st_nlink==0\nfor fd in range(3,64):\n try: os.fstat(fd)\n except OSError: continue\n raise AssertionError('inherited descriptor')\nprint(json.dumps({'readonly':True,'private_fds':True}),flush=True)\ntime.sleep(30)").await;
    assert_eq!(
        readonly.receive().await.unwrap(),
        json!({"readonly":true,"private_fds":true}),
        "the sandbox must protect its package and private descriptors"
    );
    readonly.shutdown().await.unwrap();
    ping(&mut healthy, 0).await;
    results.push(json!({"case":"readonly-memory-source"}));
    let mut maximal:Value=serde_json::from_slice(package("maximal","print(json.dumps({'size':os.stat('/plugin/main.py').st_size}),flush=True)\ntime.sleep(30)").bytes()).unwrap();
    let program = maximal["program"].as_str().unwrap();
    let full = format!(
        "{program}#{}",
        "x".repeat(MAX_PROGRAM_BYTES - program.len() - 1)
    );
    maximal["program"] = json!(full);
    let maximal = Package::parse(&serde_json::to_vec(&maximal).unwrap()).unwrap();
    let mut worker = Worker::spawn(&pool, &maximal, &json!({}), &BTreeSet::new())
        .await
        .unwrap();
    assert_eq!(
        worker.receive().await.unwrap(),
        json!({"size":MAX_PROGRAM_BYTES}),
        "the maximum-sized program must execute without truncation"
    );
    worker.shutdown().await.unwrap();
    ping(&mut healthy, 0).await;
    results.push(json!({"case":"maximum-source-bootstrap","bytes":MAX_PROGRAM_BYTES}));
    for (id, body, expected) in [
        ("crash", "os._exit(2)", Fault::Exited),
        ("hang", "while True: pass", Fault::Deadline),
        (
            "invalid",
            "print('not-json',flush=True)",
            Fault::InvalidJson,
        ),
        (
            "oversize",
            "print('x'*70000,flush=True)",
            Fault::FrameTooLarge,
        ),
        ("memory", "data=bytearray(128*1024*1024)", Fault::Exited),
    ] {
        let mut worker = spawn(&pool, id, body).await;
        assert_eq!(
            worker.receive().await.unwrap_err(),
            expected,
            "the injected failure must map to its specific fault"
        );
        let stats = worker.resources().unwrap();
        if id == "memory" {
            assert!(
                stats.oom_kills > 0,
                "the memory fixture must hit its owned cgroup memory limit"
            );
        }
        results.push(json!({"case":id,"fault":format!("{expected:?}"),"memory_peak_bytes":stats.memory_peak_bytes,"oom_kills":stats.oom_kills}));
        worker.shutdown().await.unwrap();
        ping(&mut healthy, results.len() as u64).await;
    }
    let mut flood = spawn(
        &pool,
        "flood",
        "for _ in range(100): print('{}',flush=True)\ntime.sleep(30)",
    )
    .await;
    let mut rejected = false;
    for _ in 0..100 {
        if let Err(error) = flood.receive().await {
            assert_eq!(
                error,
                Fault::Flood,
                "protocol floods must terminate with the flood fault"
            );
            rejected = true;
            break;
        }
    }
    assert!(
        rejected,
        "the supervisor must reject the injected protocol violation"
    );
    flood.shutdown().await.unwrap();
    ping(&mut healthy, 100).await;
    results.push(json!({"case":"flood","fault":"Flood"}));
    let mut processes=spawn(&pool,"processes","while True:\n try:\n  pid=os.fork()\n except OSError:\n  print(json.dumps({'limited':True}),flush=True)\n  break\n if pid==0:\n  time.sleep(30)\n  os._exit(0)\ntime.sleep(30)").await;
    assert_eq!(
        processes.receive().await.unwrap(),
        json!({"limited":true}),
        "the process fixture must report process isolation"
    );
    let stats = processes.resources().unwrap();
    assert!(
        stats.process_limit_hits > 0,
        "the process fixture must hit its owned process limit"
    );
    processes.shutdown().await.unwrap();
    ping(&mut healthy, 101).await;
    results.push(json!({"case":"processes","limit_hits":stats.process_limit_hits}));
    let mut blocked = spawn(
        &pool,
        "blocked",
        "print(json.dumps({'ready':True}),flush=True)\ntime.sleep(30)",
    )
    .await;
    assert_eq!(
        blocked.receive().await.unwrap(),
        json!({"ready":true}),
        "the worker must finish its startup handshake"
    );
    let message = json!({"data":"x".repeat(60000)});
    let mut rejected = false;
    for _ in 0..32 {
        if let Err(error) = blocked.send(&message).await {
            assert_eq!(
                error,
                Fault::InputBlocked,
                "blocked stdin must terminate with the input fault"
            );
            rejected = true;
            break;
        }
    }
    assert!(
        rejected,
        "the supervisor must reject the injected protocol violation"
    );
    blocked.shutdown().await.unwrap();
    ping(&mut healthy, 102).await;
    results.push(json!({"case":"blocked-input","fault":"InputBlocked"}));
    let mut dribble = spawn(
        &pool,
        "dribble",
        "sys.stdout.write('{');sys.stdout.flush();time.sleep(30)",
    )
    .await;
    let started = Instant::now();
    let mut rejected = false;
    for _ in 0..45 {
        if let Ok(result) =
            tokio::time::timeout(Duration::from_millis(100), dribble.receive()).await
        {
            assert_eq!(
                result.unwrap_err(),
                Fault::Deadline,
                "silent startup must terminate at its deadline"
            );
            rejected = true;
            break;
        }
    }
    assert!(
        rejected && started.elapsed() < Duration::from_secs(5),
        "protocol dribbling must not extend the overall deadline"
    );
    dribble.shutdown().await.unwrap();
    ping(&mut healthy, 103).await;
    results.push(json!({"case":"cancelled-read-deadline","fault":"Deadline"}));
    for _ in 0..8 {
        let worker = spawn(&pool, "dropped", "time.sleep(30)").await;
        drop(worker);
        reclaim(&pool, &pool_path, 1).await;
        ping(&mut healthy, 104).await;
    }
    results.push(json!({"case":"drop-reclamation","cycles":8}));
    let silent=Package::parse(&serde_json::to_vec(&json!({"format":1,"id":"silent","name":"Silent fixture","version":"1","description":"Cancelled startup fixture","permissions":[],"elements":[],"settings":[],"program":"import time\ntime.sleep(30)\n"})).unwrap()).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            Worker::spawn(&pool, &silent, &json!({}), &BTreeSet::new())
        )
        .await
        .is_err(),
        "startup cancellation must return before the worker timeout"
    );
    reclaim(&pool, &pool_path, 1).await;
    ping(&mut healthy, 105).await;
    results.push(json!({"case":"cancelled-start-reclamation"}));
    supervisor_death(&nonce, pool_path.parent().unwrap()).await;
    ping(&mut healthy, 106).await;
    results.push(json!({"case":"abrupt-supervisor-exit"}));
    healthy.shutdown().await.unwrap();
    reclaim(&pool, &pool_path, 0).await;
    drop(pool);
    println!(
        "{}",
        json!({"isolated_worker_cases":results,"healthy_worker_survived_all":true,"supervisor_survived":true})
    );
}
