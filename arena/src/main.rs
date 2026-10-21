//! Arène des bots Traversal : le bac à sable où jouent les bots, en local (`duel`,
//! `check`) comme sur le site (`work`).
//!
//! ```text
//! traversal-arena duel mon_bot.wasm sommet [--games 20] [--verbose]   # en local
//! traversal-arena check mon_bot.wasm                                   # le bot est-il valide ?
//! traversal-arena selftest                                             # cloisonnement actif ?
//! traversal-arena work --api https://… [--max-seconds 1500] [--forever] # production
//! ```
//!
//! Un bot est un fichier `.wasm` ou un bot maison : `galet`, `sentier`, `crete`, `sommet`,
//! `montblanc` (ou `1` à `5`), lus dans `--house` (défaut `$HOUSE_DIR`, sinon `/house`).
//!
//! `work` : réclame des parties au backend, les fait jouer, renvoie les coups. Il ne
//! détient qu'un jeton (`ARENA_TOKEN`) qui ne sert qu'à ça, et le backend rejoue et
//! vérifie chaque coup avec son propre moteur. Chaque partie se joue dans un processus
//! enfant cloisonné, sans ce jeton ni le code des autres bots (`isolate.rs`).

mod game;
mod isolate;
mod sandbox;

use std::collections::HashMap;
use std::io::Read;
use std::process::exit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use traversal_core::config::{MOVE_LIMIT, SQUARES};
use traversal_core::state::NONE;
use wasmtime::{Engine, Module};

use crate::sandbox::Fault;

/// Fuel par coup en local (`duel`, `check`). En production, c'est le serveur du site qui
/// le fixe à chaque tâche (même valeur).
const DEFAULT_FUEL: u64 = 2_000_000_000;
const DEFAULT_BOARDS: &str = include_str!("../../boards.json");

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    exit(2)
}

fn house_level(name: &str) -> Option<u8> {
    Some(match name {
        "galet" | "1" => 1,
        "sentier" | "2" => 2,
        "crete" | "3" => 3,
        "sommet" | "4" => 4,
        "montblanc" | "mont-blanc" | "5" => 5,
        _ => return None,
    })
}

fn house_dir(args: &HashMap<String, String>) -> String {
    args.get("house").cloned().or_else(|| std::env::var("HOUSE_DIR").ok()).unwrap_or_else(|| "/house".into())
}

fn load(engine: &Engine, spec: &str, dir: &str) -> Module {
    let path = match house_level(spec) {
        Some(l) => format!("{dir}/{l}.wasm"),
        None => spec.to_string(),
    };
    let bytes = std::fs::read(&path).unwrap_or_else(|e| die(&format!("{path} : {e}")));
    sandbox::compile(engine, &bytes).unwrap_or_else(|f| die(&format!("{path} : {f}")))
}

fn boards() -> Vec<[u8; SQUARES]> {
    let raw: Vec<Vec<u8>> = serde_json::from_str(DEFAULT_BOARDS).expect("boards.json");
    raw.into_iter().map(|b| b.try_into().expect("100 cases")).collect()
}

fn num<T: std::str::FromStr>(args: &HashMap<String, String>, key: &str, default: T) -> T {
    args.get(key).map_or(default, |v| v.parse().unwrap_or_else(|_| die(&format!("--{key} : valeur invalide"))))
}

/// Écart d'Elo correspondant à un score (0..1), et sa marge à 95 %.
fn elo(score: f64, games: f64) -> (f64, f64) {
    let p = score.clamp(0.001, 0.999);
    let diff = -400.0 * (1.0 / p - 1.0).log10();
    let se = (p * (1.0 - p) / games).sqrt();
    let hi = (p + 1.96 * se).clamp(0.001, 0.999);
    (diff, -400.0 * (1.0 / hi - 1.0).log10() - diff)
}

fn duel(pos: &[String], args: &HashMap<String, String>) {
    let [a_spec, b_spec] = pos else { die("usage : traversal-arena duel A B [--games N]") };
    let engine = sandbox::engine();
    let dir = house_dir(args);
    let (a, b) = (load(&engine, a_spec, &dir), load(&engine, b_spec, &dir));
    let games: usize = num(args, "games", 20);
    let fuel: u64 = num(args, "fuel", DEFAULT_FUEL);
    let seed: u64 = num(args, "seed", 1);
    let verbose = args.contains_key("verbose");
    let threads: usize = num(args, "threads", thread::available_parallelism().map_or(4, |n| n.get()));
    let boards = Arc::new(boards());
    println!("{a_spec} contre {b_spec} : {games} parties, {} M de fuel par coup", fuel / 1_000_000);

    let next = Arc::new(AtomicUsize::new(0));
    let results = Arc::new(Mutex::new(Vec::new()));
    let t0 = Instant::now();
    let workers: Vec<_> = (0..threads.min(games))
        .map(|_| {
            let (engine, a, b, next, results, boards) =
                (engine.clone(), a.clone(), b.clone(), next.clone(), results.clone(), boards.clone());
            thread::spawn(move || loop {
                let g = next.fetch_add(1, Ordering::Relaxed);
                if g >= games {
                    break;
                }
                // Parties 2k et 2k+1 : même plateau, couleurs inversées.
                let a_red = g % 2 == 0;
                let (red, blue) = if a_red { (&a, &b) } else { (&b, &a) };
                let o = game::play(&engine, red, blue, boards[(g / 2) % boards.len()], MOVE_LIMIT, seed + g as u64, fuel, verbose);
                let a_side = if a_red { 1 } else { 2 };
                let points = match o.winner {
                    None => 0.5,
                    Some(w) if w == a_side => 1.0,
                    Some(_) => 0.0,
                };
                let line = format!(
                    "partie {:>3} : {} en rouge, {} en {} demi-coups ({}){}",
                    g + 1,
                    if a_red { "A" } else { "B" },
                    match o.winner { None => "nulle".to_string(), Some(w) => format!("{} gagne", if w == a_side { "A" } else { "B" }) },
                    o.moves.len(),
                    o.reason,
                    o.fault.as_ref().map_or(String::new(), |(s, f)| format!(" — {} : {f}", if *s == a_side { "A" } else { "B" })),
                );
                let a_fuel = o.fuel[a_side as usize - 1];
                if verbose {
                    for (i, log) in o.logs.iter().enumerate() {
                        if !log.is_empty() {
                            eprintln!("[partie {} — {}]\n{log}", g + 1, if (i as u8 + 1) == a_side { "A" } else { "B" });
                        }
                    }
                }
                println!("{line}");
                results.lock().unwrap().push((points, a_fuel, o.fault.is_some()));
            })
        })
        .collect();
    for w in workers {
        w.join().expect("thread");
    }
    let res = results.lock().unwrap();
    let n = res.len() as f64;
    let score = res.iter().map(|r| r.0).sum::<f64>() / n;
    let (diff, margin) = elo(score, n);
    let wins = res.iter().filter(|r| r.0 == 1.0).count();
    let draws = res.iter().filter(|r| r.0 == 0.5).count();
    println!(
        "\nA : {wins} V / {draws} N / {} D — score {:.1} % — Elo(A) − Elo(B) ≈ {diff:+.0} ± {margin:.0}",
        res.len() - wins - draws,
        100.0 * score
    );
    println!(
        "fuel moyen de A : {:.0} M par coup ({:.0} % du budget) · {} partie(s) perdue(s) sur faute · {:.1} s",
        res.iter().map(|r| r.1 as f64).sum::<f64>() / n / 1e6,
        100.0 * res.iter().map(|r| r.1 as f64).sum::<f64>() / n / fuel as f64,
        res.iter().filter(|r| r.2).count(),
        t0.elapsed().as_secs_f64()
    );
}

/// Validation d'un bot : il se charge et joue une partie complète de chaque couleur contre
/// Galet sans faute (perdre est permis).
fn validate(engine: &Engine, module: &Module, galet: &Module, heights: [u8; SQUARES], seed: u64, fuel: u64) -> Result<(), String> {
    for red in [true, false] {
        let (r, b) = if red { (module, galet) } else { (galet, module) };
        let o = game::play(engine, r, b, heights, MOVE_LIMIT, seed, fuel, false);
        let me = if red { 1 } else { 2 };
        if let Some((side, f)) = o.fault {
            if side == me {
                return Err(f.to_string());
            }
            return Err(format!("bot maison en faute : {f}"));
        }
    }
    Ok(())
}

/// Une tâche, jouée dans le processus enfant cloisonné (`isolate`) : validation d'un bot
/// (le rouge, contre Galet en bleu) ou partie.
fn play_task(engine: &Engine, job: &Value, red: &[u8], blue: &[u8]) -> Value {
    if job["kind"] == "probe" {
        return probe();
    }
    let heights: [u8; SQUARES] = job["heights"]
        .as_array()
        .and_then(|a| a.iter().map(|v| v.as_u64().map(|x| x as u8)).collect::<Option<Vec<u8>>>())
        .and_then(|v| v.try_into().ok())
        .unwrap_or_else(|| die("tâche invalide : heights"));
    let seed = job["seed"].as_u64().unwrap_or(1);
    let fuel = job["fuel"].as_u64().unwrap_or(DEFAULT_FUEL);
    let move_limit = job["moveLimit"].as_u64().unwrap_or(MOVE_LIMIT as u64) as u16;
    let (red, blue) = (sandbox::compile(engine, red), sandbox::compile(engine, blue));
    if job["kind"] == "check" {
        let res = match (red, blue) {
            (Ok(bot), Ok(galet)) => validate(engine, &bot, &galet, heights, seed, fuel),
            (Err(f), _) => Err(f.to_string()),
            (_, Err(f)) => Err(format!("bot maison : {f}")),
        };
        return json!({"ok": res.is_ok(), "error": res.err()});
    }
    let fault_of = |side: u8, f: Fault| json!({"moves": [], "fault": {"side": side, "kind": f.code(), "detail": f.to_string()}});
    let (red, blue) = match (red, blue) {
        (Ok(r), Ok(b)) => (r, b),
        (Err(f), _) => return fault_of(1, f),
        (_, Err(f)) => return fault_of(2, f),
    };
    let o = game::play(engine, &red, &blue, heights, move_limit, seed, fuel, false);
    json!({
        "moves": o.moves.iter().map(wire_move).collect::<Vec<_>>(),
        "fault": o.fault.map(|(side, f)| json!({"side": side, "kind": f.code(), "detail": f.to_string()})),
        "fuel": o.fuel,
    })
}

/// Autotest : ce que le processus enfant cloisonné arrive à faire (il ne doit rien pouvoir).
fn probe() -> Value {
    let file = ["/etc/hostname", "/proc/self/status", "/tmp"].iter().any(|p| std::fs::File::open(p).is_ok());
    let net = std::net::UdpSocket::bind("127.0.0.1:0").is_ok();
    let spawn = std::process::Command::new("/bin/true").status().is_ok();
    json!({"file": file, "net": net, "spawn": spawn, "env": std::env::vars_os().count()})
}

fn wire_move(m: &traversal_core::Move) -> Value {
    let idx = |v: u8| if v == NONE { -1 } else { v as i64 };
    json!([idx(m.piece), m.kind.name(), idx(m.to)])
}

fn house_bytes(dir: &str, level: u64) -> Vec<u8> {
    let path = format!("{dir}/{level}.wasm");
    std::fs::read(&path).unwrap_or_else(|e| die(&format!("{path} : {e}")))
}

/// Validation d'un bot, exactement comme en production (processus enfant cloisonné).
fn check(pos: &[String], args: &HashMap<String, String>) {
    let [path] = pos else { die("usage : traversal-arena check bot.wasm") };
    let bytes = std::fs::read(path).unwrap_or_else(|e| die(&format!("{path} : {e}")));
    let job = json!({"kind": "check", "heights": boards()[0].to_vec(), "seed": 1, "fuel": num(args, "fuel", DEFAULT_FUEL)});
    let res = isolate::run(&job, &bytes, &house_bytes(&house_dir(args), 1))
        .unwrap_or_else(|e| json!({"ok": false, "error": format!("limites dépassées : {e}")}));
    match res["error"].as_str() {
        None => println!("✅ {path} : valide ({} Ko)", bytes.len() / 1024),
        Some(e) => die(&format!("❌ {path} : {e}")),
    }
}

// --- Production : file de parties du backend --------------------------------------------

struct Api {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl Api {
    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.agent
            .post(&format!("{}/api/arena/runner/{path}", self.base))
            .set("x-arena-token", &self.token)
            .send_json(body)
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())
    }

    fn wasm(&self, sha: &str) -> Result<Vec<u8>, String> {
        let res = self
            .agent
            .get(&format!("{}/api/arena/runner/wasm/{sha}", self.base))
            .set("x-arena-token", &self.token)
            .call()
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        res.into_reader().take(sandbox::MAX_MODULE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        // Le fichier reçu est bien celui qui a été envoyé (et validé) : empreinte vérifiée.
        let got = format!("{:x}", Sha256::digest(&bytes));
        if got != sha {
            return Err(format!("empreinte inattendue pour {sha}"));
        }
        Ok(bytes)
    }
}

/// Modules (octets, vérifiés) par empreinte ou `house:N`. Le runner ne compile ni
/// n'exécute rien lui-même : tout se passe dans les processus enfants.
struct Modules {
    house: String,
    cache: HashMap<String, Arc<Vec<u8>>>,
}

impl Modules {
    fn get(&mut self, api: &Api, who: &Value) -> Arc<Vec<u8>> {
        let key = match (who["house"].as_u64(), who["sha256"].as_str()) {
            (Some(l), _) => format!("house:{l}"),
            (None, Some(sha)) => sha.to_string(),
            _ => die("tâche invalide : joueur inconnu"),
        };
        let house = self.house.clone();
        self.cache
            .entry(key.clone())
            .or_insert_with(|| {
                Arc::new(match who["house"].as_u64() {
                    Some(l) => house_bytes(&house, l),
                    None => api.wasm(&key).unwrap_or_else(|e| die(&format!("téléchargement de {key} : {e}"))),
                })
            })
            .clone()
    }
}

/// Résultat d'une tâche, ou None si le processus enfant n'a rien rendu (la tâche sera
/// reprise plus tard par le backend, puis abandonnée après 3 essais).
fn run_job(api: &Api, mods: &mut Modules, job: &Value) -> Option<Value> {
    let (red, blue) = (mods.get(api, &job["red"]), mods.get(api, &job["blue"]));
    match isolate::run(job, &red, &blue) {
        Ok(v) => Some(v),
        // Validation : le bot a fait tomber son processus (compilation trop longue ou trop
        // gourmande, ou pire). Le runner a passé son autotest : la faute est au bot.
        Err(e) if job["kind"] == "check" => Some(json!({"ok": false, "error": format!("limites dépassées : {e}")})),
        Err(e) => {
            eprintln!("tâche {} : {e}", job["id"]);
            None
        }
    }
}

/// Avant toute tâche : une partie Galet contre Galet dans un processus enfant cloisonné.
/// Si le cloisonnement empêche de jouer (appel système manquant au filtre…), on s'arrête
/// au lieu de faire perdre ou refuser des bots qui n'y sont pour rien.
fn self_test(house: &str) {
    let fail = |msg: String| die(&format!("autotest du cloisonnement : {msg}"));
    match isolate::run(&json!({"kind": "probe"}), &[], &[]) {
        Ok(v) if v == json!({"file": false, "net": false, "spawn": false, "env": 0}) => {}
        Ok(v) => fail(format!("le processus enfant n'est pas verrouillé : {v}")),
        Err(e) => fail(e),
    }
    let galet = house_bytes(house, 1);
    let job = json!({"kind": "game", "heights": boards()[0].to_vec(), "seed": 1, "fuel": DEFAULT_FUEL});
    match isolate::run(&job, &galet, &galet) {
        Ok(v) if v["fault"].is_null() && v["moves"].as_array().is_some_and(|m| !m.is_empty()) => {}
        Ok(v) => fail(format!("partie anormale {v}")),
        Err(e) => fail(e),
    }
}

fn work(args: &HashMap<String, String>) {
    let base = args.get("api").cloned().or_else(|| std::env::var("ARENA_API").ok()).unwrap_or_else(|| die("--api manquant"));
    let token = std::env::var("ARENA_TOKEN").unwrap_or_else(|_| die("ARENA_TOKEN manquant"));
    let max = Duration::from_secs(num(args, "max-seconds", 1500));
    let forever = args.contains_key("forever");
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).build();
    let api = Api { base: base.trim_end_matches('/').to_string(), token, agent };
    let mut mods = Modules { house: house_dir(args), cache: HashMap::new() };
    self_test(&mods.house);
    let t0 = Instant::now();
    let (mut played, mut errors) = (0u32, 0u32);
    while t0.elapsed() < max {
        let job = match api.post("claim", json!({})) {
            Ok(v) => {
                errors = 0;
                v["job"].clone()
            }
            Err(e) => {
                errors += 1;
                eprintln!("backend injoignable ({errors}) : {e}");
                if errors >= 5 {
                    exit(1);
                }
                thread::sleep(Duration::from_secs(5 * errors as u64));
                continue;
            }
        };
        if job.is_null() {
            if forever {
                thread::sleep(Duration::from_secs(10));
                continue;
            }
            break;
        }
        let id = job["id"].as_str().unwrap_or_default().to_string();
        let Some(result) = run_job(&api, &mut mods, &job) else { continue };
        match api.post(&format!("jobs/{id}"), result) {
            Ok(_) => played += 1,
            Err(e) => eprintln!("résultat de {id} refusé : {e}"),
        }
    }
    println!("{played} tâche(s) traitée(s) en {:.0} s", t0.elapsed().as_secs_f64());
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first() else { die("usage : traversal-arena duel|check|work …") };
    let (mut pos, mut args) = (Vec::new(), HashMap::new());
    let mut it = argv[1..].iter().peekable();
    while let Some(a) = it.next() {
        match a.strip_prefix("--") {
            Some(key) => {
                let val = match it.peek() {
                    Some(v) if !v.starts_with("--") => it.next().unwrap().clone(),
                    _ => String::new(), // drapeau sans valeur (--verbose, --forever)
                };
                args.insert(key.to_string(), val);
            }
            None => pos.push(a.clone()),
        }
    }
    match cmd.as_str() {
        "duel" => duel(&pos, &args),
        "check" => check(&pos, &args),
        "work" => work(&args),
        "selftest" => {
            self_test(&house_dir(&args));
            println!("✅ cloisonnement actif");
        }
        // Processus enfant de `work` et `check` (voir isolate.rs) : pas pour un humain.
        "play" => isolate::child(num(&args, "cpu", 300), play_task),
        _ => die(&format!("commande inconnue : {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bot écrit à la main : joue toujours le coup `pick` (ou exécute `body`).
    fn wat(body: &str) -> String {
        format!(
            r#"(module
                 (import "traversal" "fuel_left" (func $fuel (result i64)))
                 (memory (export "memory") 1)
                 (func (export "traversal_buffer") (result i32) (i32.const 1024))
                 (func (export "traversal_move") (param $len i32) (result i32) {body}))"#
        )
    }

    fn module(engine: &Engine, src: &str) -> Result<Module, Fault> {
        sandbox::compile(engine, src.as_bytes())
    }

    fn duel_of(red: &str, blue: &str) -> game::Outcome {
        let engine = sandbox::engine();
        let (r, b) = (module(&engine, red).unwrap(), module(&engine, blue).unwrap());
        game::play(&engine, &r, &b, boards()[0], MOVE_LIMIT, 1, 10_000_000, false)
    }

    #[test]
    fn first_move_bots_finish_a_game() {
        let o = duel_of(&wat("(i32.const 0)"), &wat("(i32.const 0)"));
        assert!(o.fault.is_none());
        assert!(!o.moves.is_empty());
    }

    #[test]
    fn infinite_loop_runs_out_of_fuel() {
        let o = duel_of(&wat("(loop $l (br $l)) (i32.const 0)"), &wat("(i32.const 0)"));
        assert!(matches!(o.fault, Some((1, Fault::OutOfFuel))));
        assert_eq!((o.winner, o.reason.as_str()), (Some(2), "timeout"));
    }

    #[test]
    fn bad_index_and_trap_lose() {
        let o = duel_of(&wat("(i32.const 0)"), &wat("(i32.const 9999)"));
        assert!(matches!(o.fault, Some((2, Fault::Illegal(9999)))));
        let o = duel_of(&wat("(unreachable)"), &wat("(i32.const 0)"));
        assert!(matches!(o.fault, Some((1, Fault::Crash(_)))));
    }

    #[test]
    fn memory_is_capped() {
        // 64 Mo = 1024 pages : en demander 2000 échoue (memory.grow renvoie -1), sans planter l'hôte.
        let o = duel_of(&wat("(drop (memory.grow (i32.const 2000))) (i32.const 0)"), &wat("(i32.const 0)"));
        assert!(o.fault.is_none());
    }

    #[test]
    fn foreign_imports_are_refused() {
        let engine = sandbox::engine();
        let src = r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
                     (memory (export "memory") 1)
                     (func (export "traversal_buffer") (result i32) (i32.const 0))
                     (func (export "traversal_move") (param i32) (result i32) (i32.const 0)))"#;
        assert!(matches!(module(&engine, src), Err(Fault::Setup(m)) if m.contains("import interdit")));
        assert!(matches!(module(&engine, "(module (memory (export \"memory\") 1))"), Err(Fault::Setup(_))));
    }
}

