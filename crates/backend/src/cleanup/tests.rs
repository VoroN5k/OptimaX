use super::*;
use optimax_cleanup::system::ToolOutput;
use optimax_cleanup::MemFs;
use optimax_core::service::MemServices;
use optimax_core::CoreError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const NOW: i64 = 1_800_000_000;
const OLD: i64 = NOW - 30 * 86_400;
const TEMP: &str = r"C:\Users\Test\AppData\Local\Temp";
const DX: &str = r"C:\Users\Test\AppData\Local\NVIDIA\DXCache";

fn leak<T>(v: T) -> &'static T {
    Box::leak(Box::new(v))
}

/// Точка відновлення, що рахує спроби.
struct Restore {
    ok: bool,
    calls: AtomicUsize,
}

impl RestorePointApi for Restore {
    fn create(&self, _description: &str) -> Result<i64, CoreError> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        if self.ok {
            Ok(n as i64 + 1)
        } else {
            Err(CoreError::Os {
                api: "SRSetRestorePointW",
                code: 1058,
            })
        }
    }
}

struct Tools;

impl SystemTools for Tools {
    fn dism_component_cleanup(&self) -> Result<ToolOutput, CleanupError> {
        Ok(ToolOutput {
            code: 0,
            output: "ok".into(),
        })
    }

    fn set_hibernate(&self, _on: bool) -> Result<ToolOutput, CleanupError> {
        Ok(ToolOutput {
            code: 0,
            output: "ok".into(),
        })
    }

    fn reveal(&self, _path: &str) -> Result<(), CleanupError> {
        Ok(())
    }
}

struct Platform {
    sid: String,
    fs: &'static MemFs,
    services: MemServices,
    restore: &'static Restore,
    clients: &'static Mutex<Vec<u32>>,
}

impl CleanupPlatform for Platform {
    fn env(&self) -> Env {
        Env::for_tests()
    }
    fn data_dir(&self) -> CleanResult<String> {
        Ok(format!(r"C:\ProgramData\OptimaX\{}", self.sid))
    }
    fn dir_on_drive(&self, drive: &str) -> CleanResult<String> {
        let d = format!(r"{drive}OptimaX\{}", self.sid);
        self.fs.add_dir(&d);
        Ok(d)
    }
    fn system_drive(&self) -> String {
        r"C:\".into()
    }
    fn free_bytes(&self, _drive: &str) -> Option<u64> {
        Some(1 << 30)
    }
    fn now_unix(&self) -> i64 {
        NOW
    }
    fn fs(&self) -> &dyn FsApi {
        self.fs
    }
    fn services(&self) -> &dyn ServiceApi {
        &self.services
    }
    fn restore(&self) -> &dyn RestorePointApi {
        self.restore
    }
    fn tools(&self) -> &dyn SystemTools {
        &Tools
    }
    fn set_client(&self, pid: u32) {
        self.clients.lock().unwrap().push(pid);
    }
}

struct World {
    fs: &'static MemFs,
    restore: &'static Restore,
    clients: &'static Mutex<Vec<u32>>,
}

fn world(restore_ok: bool) -> World {
    let fs = leak(MemFs::new());
    fs.add_file(&format!(r"{TEMP}\a.tmp"), 100, OLD)
        .add_file(&format!(r"{TEMP}\new.tmp"), 7, NOW)
        .add_file(&format!(r"{DX}\shader.nvph"), 400, OLD)
        .add_file(r"C:\Users\Test\Documents\thesis.docx", 10, OLD)
        .add_dir(r"C:\ProgramData\OptimaX\S-1-5-21-1")
        .add_dir(r"C:\ProgramData\OptimaX\S-1-5-21-2");
    World {
        fs,
        restore: leak(Restore {
            ok: restore_ok,
            calls: AtomicUsize::new(0),
        }),
        clients: leak(Mutex::new(Vec::new())),
    }
}

fn cleaner(w: &World, sid: &str) -> Cleaner {
    Cleaner::new(Box::new(Platform {
        sid: sid.into(),
        fs: w.fs,
        services: MemServices::new(&[]),
        restore: w.restore,
        clients: w.clients,
    }))
}

fn q() -> QuarantineSpec {
    QuarantineSpec {
        drive: None,
        days: 7,
    }
}

fn scan_now(c: &Cleaner, mode: ScanMode) -> ScanReport {
    c.start_scan(r"C:\", false, mode)
        .unwrap()
        .run(c, &|_| {}, &|_| {})
}

#[test]
fn scan_dry_run_execute_and_restore_from_quarantine() {
    let w = world(true);
    let c = cleaner(&w, "S-1-5-21-1");
    // Без скану — нічого вибрати.
    assert!(matches!(
        c.dry_run(&[TEMP.into()], &q()),
        Err(CleanError::NotFound(_))
    ));
    let report = scan_now(&c, ScanMode::Screen);
    assert!(report.items.iter().any(|i| i.path == TEMP && i.selectable));
    // Шлях не з останнього скану (чи з чорного списку) не вибирається.
    for p in [r"C:\Users\Test\Documents", r"C:\Windows\System32"] {
        assert!(
            matches!(c.dry_run(&[p.into()], &q()), Err(CleanError::Forbidden(_))),
            "{p}"
        );
    }
    let before = w.fs.mutation_count();
    let view = c.dry_run(&[TEMP.into(), DX.into()], &q()).unwrap();
    assert_eq!(view.delete_bytes, 100, "новий файл лишається");
    assert_eq!(view.quarantine_bytes, 400);
    assert!(view.restore_point_pending);
    assert!(view.quarantine_same_volume);
    assert_eq!(
        w.fs.mutation_count(),
        before,
        "сухий прогін нічого не змінює"
    );

    let job = c.start_exec(view.plan_id, false, 1, &q()).unwrap();
    let done = job.run(&c, &|_| {}).unwrap();
    assert_eq!(done.record.freed_bytes, 100);
    assert_eq!(done.record.quarantined_bytes, 400);
    assert!(!w.fs.exists(&format!(r"{TEMP}\a.tmp")));
    assert!(w.fs.exists(&format!(r"{TEMP}\new.tmp")));
    assert!(w.fs.exists(r"C:\Users\Test\Documents\thesis.docx"));
    assert_eq!(w.restore.calls.load(Ordering::Relaxed), 1);
    // План виконується один раз.
    assert!(matches!(
        c.start_exec(view.plan_id, false, 1, &q()),
        Err(CleanError::NotFound(_))
    ));

    let qs = c.quarantine(&q()).unwrap();
    assert_eq!(qs.batches.len(), 1);
    assert!(qs.on_system_drive);
    let record = c.batch(&qs.batches[0].id, true, &q()).unwrap();
    assert_eq!(record.restored_bytes, 400);
    assert_eq!(w.fs.len_of(&format!(r"{DX}\shader.nvph")), Some(400));
    assert!(c.quarantine(&q()).unwrap().batches.is_empty());
    let kinds: Vec<RunKind> = c
        .journal_records()
        .unwrap()
        .iter()
        .map(|r| r.kind)
        .collect();
    assert_eq!(kinds, vec![RunKind::Clean, RunKind::Restore]);
}

#[test]
fn without_restore_point_asks_and_new_session_tries_again() {
    let w = world(false);
    let c = cleaner(&w, "S-1-5-21-1");
    scan_now(&c, ScanMode::Screen);
    let view = c.dry_run(&[TEMP.into()], &q()).unwrap();
    let before = w.fs.mutations().len();
    // Точку не створено — питаємо користувача; спроба лишає запис у журналі.
    assert!(matches!(
        c.start_exec(view.plan_id, false, 1, &q()),
        Err(CleanError::RestorePoint(_))
    ));
    let journal = r"C:\ProgramData\OptimaX\S-1-5-21-1\cleanup-journal.json";
    assert!(
        w.fs.mutations()[before..]
            .iter()
            .all(|m| m.contains(journal)),
        "лише журнал: {:?}",
        &w.fs.mutations()[before..]
    );
    assert!(w.fs.exists(&format!(r"{TEMP}\a.tmp")));
    let records = c.journal_records().unwrap();
    assert!(records[0].note.as_deref().unwrap().contains("не виконано"));
    // Згода — очищення без точки; у тій самій сесії більше не питаємо.
    c.start_exec(view.plan_id, true, 1, &q())
        .unwrap()
        .run(&c, &|_| {})
        .unwrap();
    c.hibernate(true, false, 1).unwrap();
    c.dism(false, 1).unwrap();
    assert_eq!(w.restore.calls.load(Ordering::Relaxed), 2);
    // Новий запуск програми — нова спроба створити точку.
    assert!(matches!(c.dism(false, 2), Err(CleanError::RestorePoint(_))));
    assert_eq!(w.restore.calls.load(Ordering::Relaxed), 3);
}

#[test]
fn busy_and_background_scan_keeps_screen_report() {
    let w = world(true);
    let c = cleaner(&w, "S-1-5-21-1");
    let job = c.start_scan(r"C:\", false, ScanMode::Screen).unwrap();
    assert!(c.start_scan(r"C:\", false, ScanMode::Background).is_err());
    assert!(c.dism(true, 1).is_err(), "скан ще триває");
    job.run(&c, &|_| {}, &|_| {});
    assert!(matches!(
        c.start_scan(r"C:\Windows", false, ScanMode::Screen),
        Err(CleanError::Forbidden(_))
    ));
    let view = c.dry_run(&[TEMP.into()], &q()).unwrap();
    // Фоновий скан не забирає звіт і план екрана.
    let job = c.start_scan(r"C:\", true, ScanMode::Background).unwrap();
    assert!(!job.keeps());
    job.run(&c, &|_| {}, &|_| panic!("проміжний звіт фонового скану"));
    assert!(!c.last_scan().unwrap().report.full);
    assert!(c.start_exec(view.plan_id, true, 1, &q()).is_ok());
}

fn ok(r: Option<Reply>) -> serde_json::Value {
    match r {
        Some(Reply::Json(v)) => v,
        other => panic!("{other:?}"),
    }
}

fn result<T: serde::de::DeserializeOwned>(r: Option<Reply>) -> CleanResult<T> {
    serde_json::from_value(ok(r)).unwrap()
}

fn poll_until(h: &CleanupHost, sid: &str, done: impl Fn(&CleanupEvents) -> bool) -> CleanupEvents {
    let start = Instant::now();
    loop {
        let e: CleanupEvents = result(h.handle(9, sid, &Command::CleanupPoll)).unwrap();
        if done(&e) {
            return e;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "не дочекались");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn host_keeps_users_apart_and_reports_through_poll() {
    let w = world(true);
    let (fs, restore, clients) = (w.fs, w.restore, w.clients);
    let h = CleanupHost::new(Box::new(move |sid: &str| {
        Ok(Cleaner::new(Box::new(Platform {
            sid: sid.into(),
            fs,
            services: MemServices::new(&[]),
            restore,
            clients,
        })))
    }));
    let (a, b) = ("S-1-5-21-1", "S-1-5-21-2");
    assert!(h.handle(9, a, &Command::Ping).is_none(), "не про очищення");
    let start = Command::CleanupScanStart {
        drive: r"C:\".into(),
        full: false,
        background: false,
    };
    result::<()>(h.handle(9, a, &start)).unwrap();
    let e = poll_until(&h, a, |e| e.scan_done.is_some());
    let done = e.scan_done.unwrap();
    assert!(done.kept);
    assert!(done.scan.report.items.iter().any(|i| i.path == TEMP));
    // Інший користувач не бачить чужого скану й не може вибрати з нього.
    let dry = |paths: Vec<String>| Command::CleanupDryRun {
        paths,
        quarantine_drive: None,
        quarantine_days: 7,
    };
    assert!(
        result::<Option<LastScan>>(h.handle(9, b, &Command::CleanupLastScan))
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        result::<DryRunView>(h.handle(9, b, &dry(vec![TEMP.into()]))),
        Err(CleanError::NotFound(_))
    ));
    let view: DryRunView = result(h.handle(9, a, &dry(vec![TEMP.into()]))).unwrap();
    let exec = Command::CleanupExecute {
        plan_id: view.plan_id,
        allow_without_restore_point: false,
        session: 1,
        quarantine_drive: None,
        quarantine_days: 7,
    };
    // Номер плану чужого користувача не діє.
    assert!(matches!(
        result::<()>(h.handle(9, b, &exec)),
        Err(CleanError::NotFound(_))
    ));
    result::<()>(h.handle(9, a, &exec)).unwrap();
    let e = poll_until(&h, a, |e| e.exec_done.is_some());
    let done = e.exec_done.unwrap().unwrap();
    assert_eq!(done.record.freed_bytes, 100);
    assert!(!fs.exists(&format!(r"{TEMP}\a.tmp")));
    // Журнали окремі: у кожного користувача — свій.
    let ja: Vec<JournalRecord> = result(h.handle(9, a, &Command::CleanupJournal)).unwrap();
    let jb: Vec<JournalRecord> = result(h.handle(9, b, &Command::CleanupJournal)).unwrap();
    assert_eq!((ja.len(), jb.len()), (1, 0));
    // Перед кожною командою служба каже платформі, хто просить (теки правил — його).
    assert!(clients.lock().unwrap().iter().all(|p| *p == 9));
    // Помилка правил долітає як помилка, а не як розрив.
    assert!(matches!(
        result::<Option<GrowthView>>(h.handle(
            9,
            a,
            &Command::CleanupGrowth {
                drive: r"C:\Windows".into()
            }
        )),
        Err(CleanError::Forbidden(_))
    ));
}
