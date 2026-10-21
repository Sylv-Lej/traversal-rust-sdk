//! Cloisonnement : chaque partie (ou validation) se joue dans un processus enfant
//! (`traversal-arena play`), en plus du bac à sable WebAssembly.
//!
//! Si un bot trouvait une faille de wasmtime, il se retrouverait dans ce processus enfant,
//! qui n'a :
//!
//! - **aucun secret** : environnement vidé (pas d'`ARENA_TOKEN`) ;
//! - **que les deux modules de sa partie**, reçus sur l'entrée standard (pas le cache du
//!   runner, donc pas le code des autres bots) ;
//! - **presque aucun appel système** (seccomp, Linux) : mémoire, lecture/écriture sur les
//!   descripteurs déjà ouverts, signaux, sortie. Ni fichier, ni réseau, ni processus ;
//! - **des ressources bornées** (`setrlimit`) : temps CPU, mémoire, pas de fichier core ;
//!   le parent le tue aussi au-delà d'un délai. La compilation est donc bornée elle aussi :
//!   un module construit pour épuiser le compilateur échoue à la validation.
//!
//! Protocole : une ligne JSON (la tâche, avec la taille des deux modules), puis les octets
//! du module rouge, puis ceux du bleu. Réponse : une ligne JSON sur la sortie standard.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::sandbox::MAX_MODULE_BYTES;

/// Mémoire du processus enfant : 2 bots × 64 Mo, les modules compilés et le compilateur.
const DATA_LIMIT: u64 = 400 * 1024 * 1024;

/// Délai d'une tâche : temps CPU (`RLIMIT_CPU`) et temps réel (le parent tue l'enfant).
pub fn timeout_for(kind: &str) -> Duration {
    // Validation : 2 parties ; une partie : ≤ 200 demi-coups × 2 milliards de fuel.
    Duration::from_secs(if kind == "check" { 120 } else { 300 })
}

/// Côté parent : joue la tâche dans un processus enfant ; `Err` si l'enfant n'a pas rendu
/// de réponse (tué : délai, mémoire, appel système interdit…).
pub fn run(header: &Value, red: &[u8], blue: &[u8]) -> Result<Value, String> {
    let kind = header["kind"].as_str().unwrap_or("game");
    let limit = timeout_for(kind);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg("play")
        .arg("--cpu")
        .arg(limit.as_secs().to_string())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("lancement : {e}"))?;

    let mut head = header.clone();
    head["sizes"] = json!([red.len(), blue.len()]);
    let mut input = serde_json::to_vec(&head).map_err(|e| e.to_string())?;
    input.push(b'\n');
    input.extend_from_slice(red);
    input.extend_from_slice(blue);
    let mut stdin = child.stdin.take().expect("stdin");
    // Écriture et lectures dans des fils à part : aucun blocage, quelles que soient les tailles.
    let writer = thread::spawn(move || stdin.write_all(&input));
    let mut stdout = child.stdout.take().expect("stdout");
    let reader = thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.by_ref().take(1 << 20).read_to_end(&mut out);
        out
    });
    let mut stderr = child.stderr.take().expect("stderr");
    let errors = thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stderr.by_ref().take(4096).read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    });

    let deadline = Instant::now() + limit + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("délai dépassé ({} s)", limit.as_secs()));
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    };
    let _ = writer.join();
    let out = reader.join().unwrap_or_default();
    let err = errors.join().unwrap_or_default();
    if !status.success() {
        return Err(format!("processus arrêté ({status}){}", if err.is_empty() { String::new() } else { format!(" : {}", err.trim()) }));
    }
    serde_json::from_slice(&out).map_err(|e| format!("réponse illisible : {e}"))
}

/// Côté enfant : lit la tâche, se verrouille, puis seulement compile et joue.
pub fn child(cpu_secs: u64, play: impl FnOnce(&wasmtime::Engine, &Value, &[u8], &[u8]) -> Value) {
    limit_resources(cpu_secs);
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut line = Vec::new();
    input.by_ref().take(1 << 16).read_until(b'\n', &mut line).expect("lecture de la tâche");
    let header: Value = serde_json::from_slice(&line).expect("tâche JSON");
    let size = |i: usize| header["sizes"][i].as_u64().unwrap_or(0).min(MAX_MODULE_BYTES as u64 + 1) as usize;
    let (mut red, mut blue) = (vec![0; size(0)], vec![0; size(1)]);
    input.read_exact(&mut red).expect("module rouge");
    input.read_exact(&mut blue).expect("module bleu");
    drop(input);
    // Le moteur (détection du processeur…) est prêt avant le verrouillage.
    let engine = crate::sandbox::engine();
    lockdown();
    let result = play(&engine, &header, &red, &blue);
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &result).expect("réponse");
    out.write_all(b"\n").and_then(|_| out.flush()).expect("réponse");
}

#[cfg(target_os = "linux")]
fn limit_resources(cpu_secs: u64) {
    let set = |res, soft: u64, hard: u64| {
        let lim = libc::rlimit { rlim_cur: soft, rlim_max: hard };
        // SAFETY : appel système sans pointeur conservé.
        if unsafe { libc::setrlimit(res, &lim) } != 0 {
            panic!("setrlimit {res}");
        }
    };
    set(libc::RLIMIT_CPU, cpu_secs, cpu_secs + 5);
    set(libc::RLIMIT_DATA, DATA_LIMIT, DATA_LIMIT);
    set(libc::RLIMIT_CORE, 0, 0);
    set(libc::RLIMIT_FSIZE, 0, 0);
}

#[cfg(target_os = "linux")]
fn lockdown() {
    use std::collections::BTreeMap;

    use seccompiler::{apply_filter, BpfProgram, SeccompAction, SeccompFilter};

    // Liste blanche : ce dont wasmtime et la bibliothèque standard ont besoin pour
    // calculer, lire/écrire sur les descripteurs DÉJÀ ouverts, et s'arrêter.
    #[allow(unused_mut)]
    let mut allowed = vec![
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_writev,
        libc::SYS_close,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_brk,
        libc::SYS_mmap,
        libc::SYS_munmap,
        libc::SYS_mprotect,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_membarrier,
        libc::SYS_futex,
        libc::SYS_rt_sigreturn,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_sigaltstack,
        libc::SYS_sched_yield,
        libc::SYS_getrandom,
        libc::SYS_clock_gettime,
        libc::SYS_gettid,
        libc::SYS_getpid,
        libc::SYS_tgkill,
    ];
    #[cfg(target_arch = "x86_64")]
    allowed.push(libc::SYS_arch_prctl);
    let rules: BTreeMap<i64, Vec<seccompiler::SeccompRule>> = allowed.into_iter().map(|s| (s, vec![])).collect();
    let arch = std::env::consts::ARCH.try_into().expect("architecture prise en charge par seccomp");
    // Hors liste : refusé (EPERM) plutôt que tué, pour que l'erreur soit lisible.
    let filter = SeccompFilter::new(rules, SeccompAction::Errno(libc::EPERM as u32), SeccompAction::Allow, arch)
        .expect("filtre seccomp");
    let prog: BpfProgram = filter.try_into().expect("filtre seccomp");
    apply_filter(&prog).expect("installation du filtre seccomp");
}

// Hors Linux (développement sur Mac) : pas de seccomp ni de setrlimit. Le runner de
// production tourne toujours dans l'image Linux.
#[cfg(not(target_os = "linux"))]
fn limit_resources(_cpu_secs: u64) {}

#[cfg(not(target_os = "linux"))]
fn lockdown() {}
