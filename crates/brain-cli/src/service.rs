use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use atomicwrites::{AllowOverwrite, AtomicFile};
use brain_service::ServiceLaunchConfig;

use crate::{
    install_claude_hooks, install_codex_hooks, uninstall_claude_hooks, uninstall_codex_hooks,
};

const SERVICE_TASK: &str = "AgentBrain.Service";
const BACKUP_TASK: &str = "AgentBrain.Backup";
const DRILL_TASK: &str = "AgentBrain.RestoreDrill";
const INSTALL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct ServiceInstallOptions {
    pub brain_home: PathBuf,
    pub service_executable: PathBuf,
    pub brain_executable: PathBuf,
    pub backup_root: PathBuf,
    pub drill_root: PathBuf,
    pub hook_executable: Option<PathBuf>,
    pub claude_settings: Option<PathBuf>,
    pub codex_settings: Option<PathBuf>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ServiceInstallReport {
    pub brain_home: PathBuf,
    pub service_executable: PathBuf,
    pub brain_executable: PathBuf,
    pub backup_root: PathBuf,
    pub drill_root: PathBuf,
    pub task_names: Vec<String>,
    pub hooks_installed: Vec<String>,
    pub manifest_path: PathBuf,
    pub data_preserved_on_uninstall: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ServiceTaskStatus {
    pub task_name: String,
    pub installed: bool,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ServiceStatusReport {
    pub installed: bool,
    pub manifest_path: PathBuf,
    pub tasks: Vec<ServiceTaskStatus>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ServiceUninstallReport {
    pub removed_tasks: Vec<String>,
    pub removed_hooks: Vec<String>,
    pub brain_home_preserved: bool,
    pub backup_root_preserved: bool,
    pub drill_reports_preserved: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct InstallManifest {
    schema_version: u32,
    brain_home: PathBuf,
    service_executable: PathBuf,
    brain_executable: PathBuf,
    backup_root: PathBuf,
    drill_root: PathBuf,
    hook_executable: Option<PathBuf>,
    claude_settings: Option<PathBuf>,
    codex_settings: Option<PathBuf>,
    task_names: Vec<String>,
    installed_at: time::OffsetDateTime,
}

trait TaskScheduler {
    fn create_from_xml(&self, task_name: &str, xml_path: &Path) -> Result<()>;
    fn run(&self, task_name: &str) -> Result<()>;
    fn end(&self, task_name: &str) -> Result<()>;
    fn delete(&self, task_name: &str) -> Result<()>;
    fn query(&self, task_name: &str) -> Result<Option<String>>;
}

pub fn install_windows_service(options: ServiceInstallOptions) -> Result<ServiceInstallReport> {
    install_windows_service_with(
        &SystemTaskScheduler,
        options,
        time::OffsetDateTime::now_utc(),
    )
}

pub fn start_windows_service(brain_home: &Path) -> Result<ServiceStatusReport> {
    let manifest = load_manifest(brain_home)?;
    SystemTaskScheduler.run(SERVICE_TASK)?;
    status_with(&SystemTaskScheduler, brain_home, Some(manifest))
}

pub fn stop_windows_service(brain_home: &Path) -> Result<ServiceStatusReport> {
    let manifest = load_manifest(brain_home)?;
    if SystemTaskScheduler.query(SERVICE_TASK)?.is_some() {
        SystemTaskScheduler.end(SERVICE_TASK)?;
    }
    status_with(&SystemTaskScheduler, brain_home, Some(manifest))
}

pub fn windows_service_status(brain_home: &Path) -> Result<ServiceStatusReport> {
    let manifest = load_manifest(brain_home).ok();
    status_with(&SystemTaskScheduler, brain_home, manifest)
}

pub fn uninstall_windows_service(brain_home: &Path) -> Result<ServiceUninstallReport> {
    uninstall_windows_service_with(&SystemTaskScheduler, brain_home)
}

fn install_windows_service_with(
    scheduler: &dyn TaskScheduler,
    mut options: ServiceInstallOptions,
    now: time::OffsetDateTime,
) -> Result<ServiceInstallReport> {
    ensure!(
        cfg!(windows),
        "Windows service packaging is available only on Windows"
    );
    options.brain_home = canonical_directory(&options.brain_home, "BRAIN_HOME")?;
    options.service_executable = canonical_file(&options.service_executable, "brain-service")?;
    options.brain_executable = canonical_file(&options.brain_executable, "brain CLI")?;
    let launch_config =
        ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&options.brain_home))?;
    ensure!(
        !launch_config.projects.is_empty(),
        "register at least one project before installing the service"
    );
    std::fs::create_dir_all(&options.backup_root)?;
    std::fs::create_dir_all(&options.drill_root)?;
    options.backup_root = std::fs::canonicalize(&options.backup_root)?;
    options.drill_root = std::fs::canonicalize(&options.drill_root)?;
    ensure_separate(&options.brain_home, &options.backup_root, "backup root")?;
    ensure_separate(&options.brain_home, &options.drill_root, "drill root")?;
    if let Some(executable) = &options.hook_executable {
        options.hook_executable = Some(canonical_file(executable, "brain-hook")?);
    }
    let runtime = options.brain_home.join("runtime");
    let task_dir = runtime.join("tasks");
    let log_dir = runtime.join("logs");
    std::fs::create_dir_all(&task_dir)?;
    std::fs::create_dir_all(&log_dir)?;
    let user_id = current_user_id()?;
    let service_xml = task_dir.join("service.xml");
    let backup_xml = task_dir.join("backup.xml");
    let drill_xml = task_dir.join("restore-drill.xml");
    write_atomic(
        &service_xml,
        service_task_xml(
            &user_id,
            &options.service_executable,
            &options.brain_home,
            &log_dir,
        )?
        .as_bytes(),
    )?;
    write_atomic(
        &backup_xml,
        hourly_task_xml(
            &user_id,
            &options.brain_executable,
            &format!(
                "--brain-home {} backup maintain --root {}",
                quoted(&options.brain_home),
                quoted(&options.backup_root)
            ),
            now,
        )?
        .as_bytes(),
    )?;
    write_atomic(
        &drill_xml,
        monthly_task_xml(
            &user_id,
            &options.brain_executable,
            &format!(
                "--brain-home {} backup drill-latest --root {} --work-root {}",
                quoted(&options.brain_home),
                quoted(&options.backup_root),
                quoted(&options.drill_root)
            ),
            now,
        )?
        .as_bytes(),
    )?;
    let task_specs = [
        (SERVICE_TASK, service_xml.as_path()),
        (BACKUP_TASK, backup_xml.as_path()),
        (DRILL_TASK, drill_xml.as_path()),
    ];
    let reinstall = manifest_path(&options.brain_home).is_file();
    if reinstall {
        load_manifest(&options.brain_home)?;
    }
    for (name, _) in &task_specs {
        ensure!(
            reinstall || scheduler.query(name)?.is_none(),
            "scheduled task {name} already exists without an AgentBrain install manifest"
        );
    }
    let mut created: Vec<&str> = Vec::new();
    for (name, path) in task_specs {
        if let Err(error) = scheduler.create_from_xml(name, path) {
            for task in created.iter().rev() {
                let _ = scheduler.delete(task);
            }
            return Err(error).with_context(|| format!("install scheduled task {name}"));
        }
        created.push(name);
    }
    let mut hooks_installed = Vec::new();
    let hook_result = (|| {
        if let (Some(settings), Some(executable)) =
            (&options.claude_settings, &options.hook_executable)
        {
            install_claude_hooks(settings, executable)?;
            hooks_installed.push("claude-code".to_owned());
        }
        if let (Some(settings), Some(executable)) =
            (&options.codex_settings, &options.hook_executable)
        {
            install_codex_hooks(settings, executable)?;
            hooks_installed.push("codex".to_owned());
        }
        Result::<()>::Ok(())
    })();
    if let Err(error) = hook_result {
        for task in created.iter().rev() {
            let _ = scheduler.delete(task);
        }
        if let (Some(settings), Some(executable)) =
            (&options.claude_settings, &options.hook_executable)
        {
            let _ = uninstall_claude_hooks(settings, executable);
        }
        if let (Some(settings), Some(executable)) =
            (&options.codex_settings, &options.hook_executable)
        {
            let _ = uninstall_codex_hooks(settings, executable);
        }
        return Err(error).context("install harness hooks");
    }
    let manifest = InstallManifest {
        schema_version: INSTALL_SCHEMA_VERSION,
        brain_home: options.brain_home.clone(),
        service_executable: options.service_executable.clone(),
        brain_executable: options.brain_executable.clone(),
        backup_root: options.backup_root.clone(),
        drill_root: options.drill_root.clone(),
        hook_executable: options.hook_executable,
        claude_settings: options.claude_settings,
        codex_settings: options.codex_settings,
        task_names: created.iter().map(|name| (*name).to_owned()).collect(),
        installed_at: now,
    };
    let manifest_path = manifest_path(&options.brain_home);
    write_atomic(&manifest_path, &serde_json::to_vec_pretty(&manifest)?)?;
    scheduler.run(SERVICE_TASK)?;
    Ok(ServiceInstallReport {
        brain_home: manifest.brain_home,
        service_executable: manifest.service_executable,
        brain_executable: manifest.brain_executable,
        backup_root: manifest.backup_root,
        drill_root: manifest.drill_root,
        task_names: manifest.task_names,
        hooks_installed,
        manifest_path,
        data_preserved_on_uninstall: true,
    })
}

fn uninstall_windows_service_with(
    scheduler: &dyn TaskScheduler,
    brain_home: &Path,
) -> Result<ServiceUninstallReport> {
    let manifest = load_manifest(brain_home)?;
    let mut removed_tasks = Vec::new();
    for task in &manifest.task_names {
        if scheduler.query(task)?.is_some() {
            if task == SERVICE_TASK {
                let _ = scheduler.end(task);
            }
            scheduler.delete(task)?;
            removed_tasks.push(task.clone());
        }
    }
    let mut removed_hooks = Vec::new();
    if let (Some(settings), Some(executable)) =
        (&manifest.claude_settings, &manifest.hook_executable)
    {
        uninstall_claude_hooks(settings, executable)?;
        removed_hooks.push("claude-code".to_owned());
    }
    if let (Some(settings), Some(executable)) =
        (&manifest.codex_settings, &manifest.hook_executable)
    {
        uninstall_codex_hooks(settings, executable)?;
        removed_hooks.push("codex".to_owned());
    }
    let runtime = brain_home.join("runtime");
    for name in ["service.xml", "backup.xml", "restore-drill.xml"] {
        let path = runtime.join("tasks").join(name);
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
    }
    let path = manifest_path(brain_home);
    if path.is_file() {
        std::fs::remove_file(path)?;
    }
    Ok(ServiceUninstallReport {
        removed_tasks,
        removed_hooks,
        brain_home_preserved: brain_home.is_dir(),
        backup_root_preserved: manifest.backup_root.is_dir(),
        drill_reports_preserved: manifest.drill_root.is_dir(),
    })
}

fn status_with(
    scheduler: &dyn TaskScheduler,
    brain_home: &Path,
    manifest: Option<InstallManifest>,
) -> Result<ServiceStatusReport> {
    let task_names = manifest
        .as_ref()
        .map(|manifest| manifest.task_names.clone())
        .unwrap_or_else(|| {
            vec![
                SERVICE_TASK.to_owned(),
                BACKUP_TASK.to_owned(),
                DRILL_TASK.to_owned(),
            ]
        });
    let mut tasks = Vec::new();
    for task_name in task_names {
        let detail = scheduler.query(&task_name)?;
        tasks.push(ServiceTaskStatus {
            task_name,
            installed: detail.is_some(),
            detail,
        });
    }
    Ok(ServiceStatusReport {
        installed: manifest.is_some() && tasks.iter().all(|task| task.installed),
        manifest_path: manifest_path(brain_home),
        tasks,
    })
}

struct SystemTaskScheduler;

impl TaskScheduler for SystemTaskScheduler {
    fn create_from_xml(&self, task_name: &str, xml_path: &Path) -> Result<()> {
        command(&[
            "/Create",
            "/TN",
            task_name,
            "/XML",
            &external(xml_path),
            "/F",
        ])
        .map(|_| ())
    }

    fn run(&self, task_name: &str) -> Result<()> {
        command(&["/Run", "/TN", task_name]).map(|_| ())
    }

    fn end(&self, task_name: &str) -> Result<()> {
        command(&["/End", "/TN", task_name]).map(|_| ())
    }

    fn delete(&self, task_name: &str) -> Result<()> {
        command(&["/Delete", "/TN", task_name, "/F"]).map(|_| ())
    }

    fn query(&self, task_name: &str) -> Result<Option<String>> {
        let output = std::process::Command::new("schtasks.exe")
            .args(["/Query", "/TN", task_name, "/FO", "LIST", "/V"])
            .output()?;
        if output.status.success() {
            Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            ))
        } else {
            Ok(None)
        }
    }
}

fn command(arguments: &[&str]) -> Result<String> {
    let output = std::process::Command::new("schtasks.exe")
        .args(arguments)
        .output()?;
    ensure!(
        output.status.success(),
        "Task Scheduler command failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn service_task_xml(
    user_id: &str,
    executable: &Path,
    brain_home: &Path,
    log_dir: &Path,
) -> Result<String> {
    task_xml(
        user_id,
        "<LogonTrigger><Enabled>true</Enabled></LogonTrigger>",
        executable,
        &format!(
            "--brain-home {} --log-dir {}",
            quoted(brain_home),
            quoted(log_dir)
        ),
        "PT0S",
        true,
    )
}

fn hourly_task_xml(
    user_id: &str,
    executable: &Path,
    arguments: &str,
    now: time::OffsetDateTime,
) -> Result<String> {
    let start = start_boundary(now)?;
    task_xml(
        user_id,
        &format!(
            "<TimeTrigger><StartBoundary>{start}</StartBoundary><Enabled>true</Enabled><Repetition><Interval>PT1H</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition></TimeTrigger>"
        ),
        executable,
        arguments,
        "PT2H",
        false,
    )
}

fn monthly_task_xml(
    user_id: &str,
    executable: &Path,
    arguments: &str,
    now: time::OffsetDateTime,
) -> Result<String> {
    let start = start_boundary(now)?;
    task_xml(
        user_id,
        &format!(
            "<CalendarTrigger><StartBoundary>{start}</StartBoundary><Enabled>true</Enabled><ScheduleByMonth><DaysOfMonth><Day>1</Day></DaysOfMonth><Months><January/><February/><March/><April/><May/><June/><July/><August/><September/><October/><November/><December/></Months></ScheduleByMonth></CalendarTrigger>"
        ),
        executable,
        arguments,
        "PT12H",
        false,
    )
}

fn task_xml(
    user_id: &str,
    trigger: &str,
    executable: &Path,
    arguments: &str,
    execution_limit: &str,
    restart: bool,
) -> Result<String> {
    let working_directory = executable
        .parent()
        .context("scheduled executable has no parent")?;
    let restart = if restart {
        "<RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure>"
    } else {
        ""
    };
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><RegistrationInfo><Author>{user}</Author></RegistrationInfo><Triggers>{trigger}</Triggers><Principals><Principal id=\"Author\"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><AllowHardTerminate>true</AllowHardTerminate><ExecutionTimeLimit>{execution_limit}</ExecutionTimeLimit>{restart}</Settings><Actions Context=\"Author\"><Exec><Command>{command}</Command><Arguments>{arguments}</Arguments><WorkingDirectory>{working}</WorkingDirectory></Exec></Actions></Task>",
        user = xml_escape(user_id),
        command = xml_escape(&external(executable)),
        arguments = xml_escape(arguments),
        working = xml_escape(&external(working_directory)),
    ))
}

fn start_boundary(now: time::OffsetDateTime) -> Result<String> {
    Ok((now + time::Duration::minutes(1))
        .replace_nanosecond(0)?
        .format(&time::format_description::well_known::Rfc3339)?)
}

fn current_user_id() -> Result<String> {
    let user = std::env::var("USERNAME").context("USERNAME is unavailable")?;
    Ok(match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.trim().is_empty() => format!("{domain}\\{user}"),
        _ => user,
    })
}

fn ensure_separate(brain_home: &Path, candidate: &Path, label: &str) -> Result<()> {
    ensure!(
        !candidate.starts_with(brain_home) && !brain_home.starts_with(candidate),
        "{label} must be separate from BRAIN_HOME"
    );
    Ok(())
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    ensure!(path.is_dir(), "{label} does not exist");
    Ok(std::fs::canonicalize(path)?)
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf> {
    ensure!(path.is_file(), "{label} executable does not exist");
    Ok(std::fs::canonicalize(path)?)
}

fn load_manifest(brain_home: &Path) -> Result<InstallManifest> {
    let manifest: InstallManifest =
        serde_json::from_slice(&std::fs::read(manifest_path(brain_home))?)?;
    ensure!(
        manifest.schema_version == INSTALL_SCHEMA_VERSION,
        "unsupported service install manifest"
    );
    ensure!(
        std::fs::canonicalize(brain_home)? == manifest.brain_home,
        "service manifest belongs to another BRAIN_HOME"
    );
    Ok(manifest)
}

fn manifest_path(brain_home: &Path) -> PathBuf {
    brain_home.join("runtime").join("install.json")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("output has no parent")?;
    std::fs::create_dir_all(parent)?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .with_context(|| format!("write {}", path.display()))
}

fn external(path: &Path) -> String {
    let rendered = path.to_string_lossy();
    rendered
        .strip_prefix(r"\\?\")
        .unwrap_or(&rendered)
        .to_owned()
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", external(path).replace('"', "\\\""))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Mutex;

    use brain_domain::{ProjectId, WorktreeId};
    use brain_service::{ServiceLaunchConfig, ServiceProjectConfig};

    use super::*;

    #[test]
    fn install_and_uninstall_manage_only_owned_tasks_and_preserve_all_data() {
        let temp = tempfile::tempdir().expect("create Windows service fixture");
        let brain_home = temp.path().join("AgentBrain");
        let project_root = temp.path().join("project");
        let backup_root = temp.path().join("AgentBrainBackups");
        let drill_root = temp.path().join("AgentBrainDrills");
        let binaries = temp.path().join("release");
        std::fs::create_dir_all(brain_home.join("runtime")).expect("brain runtime");
        std::fs::create_dir_all(&project_root).expect("project root");
        std::fs::create_dir_all(&binaries).expect("binary directory");
        let service_executable = binaries.join("brain-service.exe");
        let brain_executable = binaries.join("brain.exe");
        std::fs::write(&service_executable, "fixture").expect("service executable");
        std::fs::write(&brain_executable, "fixture").expect("brain executable");
        let mut config = ServiceLaunchConfig::new("fixture-pipe");
        config.projects.push(ServiceProjectConfig {
            project_root,
            project_id: ProjectId(uuid::Uuid::now_v7()),
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            ledger_path: brain_home.join("projects/p1/events.sqlite"),
            claude_sources: Vec::new(),
            codex_sources: Vec::new(),
            hermes_database: None,
        });
        std::fs::write(
            ServiceLaunchConfig::default_path(&brain_home),
            serde_json::to_vec_pretty(&config).expect("config JSON"),
        )
        .expect("service config");
        std::fs::write(brain_home.join("canonical-marker"), "preserve").expect("canonical marker");
        let scheduler = FakeScheduler::default();
        let install = install_windows_service_with(
            &scheduler,
            ServiceInstallOptions {
                brain_home: brain_home.clone(),
                service_executable,
                brain_executable,
                backup_root: backup_root.clone(),
                drill_root: drill_root.clone(),
                hook_executable: None,
                claude_settings: None,
                codex_settings: None,
            },
            time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000),
        )
        .expect("install tasks");
        assert_eq!(install.task_names.len(), 3);
        assert!(install.manifest_path.is_file());
        assert!(
            scheduler
                .running
                .lock()
                .expect("running lock")
                .contains(SERVICE_TASK)
        );
        let xml = scheduler.xml.lock().expect("xml lock");
        assert!(xml[SERVICE_TASK].contains("ExecutionTimeLimit>PT0S"));
        assert!(xml[SERVICE_TASK].contains("RestartOnFailure"));
        assert!(xml[BACKUP_TASK].contains("PT1H"));
        assert!(xml[DRILL_TASK].contains("ScheduleByMonth"));
        drop(xml);
        std::fs::write(backup_root.join("backup-marker"), "preserve").expect("backup marker");
        std::fs::write(drill_root.join("drill-marker"), "preserve").expect("drill marker");

        let uninstall =
            uninstall_windows_service_with(&scheduler, &brain_home).expect("uninstall tasks");
        assert_eq!(uninstall.removed_tasks.len(), 3);
        assert!(uninstall.brain_home_preserved);
        assert!(uninstall.backup_root_preserved);
        assert!(uninstall.drill_reports_preserved);
        assert_eq!(
            std::fs::read_to_string(brain_home.join("canonical-marker"))
                .expect("canonical marker after uninstall"),
            "preserve"
        );
        assert!(backup_root.join("backup-marker").is_file());
        assert!(drill_root.join("drill-marker").is_file());
        assert!(!install.manifest_path.exists());
        assert!(scheduler.tasks.lock().expect("tasks lock").is_empty());
    }

    #[derive(Default)]
    struct FakeScheduler {
        tasks: Mutex<BTreeSet<String>>,
        running: Mutex<BTreeSet<String>>,
        xml: Mutex<BTreeMap<String, String>>,
    }

    impl TaskScheduler for FakeScheduler {
        fn create_from_xml(&self, task_name: &str, xml_path: &Path) -> Result<()> {
            self.tasks
                .lock()
                .expect("tasks lock")
                .insert(task_name.to_owned());
            self.xml
                .lock()
                .expect("xml lock")
                .insert(task_name.to_owned(), std::fs::read_to_string(xml_path)?);
            Ok(())
        }

        fn run(&self, task_name: &str) -> Result<()> {
            ensure!(
                self.tasks.lock().expect("tasks lock").contains(task_name),
                "task is not installed"
            );
            self.running
                .lock()
                .expect("running lock")
                .insert(task_name.to_owned());
            Ok(())
        }

        fn end(&self, task_name: &str) -> Result<()> {
            self.running.lock().expect("running lock").remove(task_name);
            Ok(())
        }

        fn delete(&self, task_name: &str) -> Result<()> {
            self.tasks.lock().expect("tasks lock").remove(task_name);
            self.running.lock().expect("running lock").remove(task_name);
            Ok(())
        }

        fn query(&self, task_name: &str) -> Result<Option<String>> {
            Ok(self
                .tasks
                .lock()
                .expect("tasks lock")
                .contains(task_name)
                .then(|| format!("TaskName: {task_name}")))
        }
    }
}
