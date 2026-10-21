//! Bac à sable d'un bot : module WebAssembly exécuté par wasmtime.
//!
//! Le bot ne peut RIEN faire d'autre que calculer : pas de WASI (ni fichiers, ni réseau,
//! ni horloge), seules deux fonctions lui sont fournies (`traversal.fuel_left`,
//! `traversal.log`). Ses ressources sont bornées :
//!
//! - **fuel** (≈ une instruction WebAssembly) : un budget par coup, remis à neuf à chaque
//!   coup ; épuisé, le coup s'arrête et la partie est perdue. Même budget pour tous, quelle
//!   que soit la machine : l'arène classe des algorithmes, pas des processeurs ;
//! - **mémoire** : `MAX_MEMORY` au plus, une seule mémoire, pas de threads ;
//! - **taille du module** : `MAX_MODULE_BYTES`.
//!
//! Exécution déterministe (NaN canoniques, pas de SIMD « relâché ») : même graine, même
//! partie, ce qui permet de rejouer et de vérifier n'importe quelle partie.

use std::fmt;

use traversal_core::wire::{self, BUFFER_SIZE};
use traversal_core::{Move, State};
use wasmtime::{Caller, Config, Engine, ExternType, Instance, Linker, Memory, Module, Store, StoreLimits,
               StoreLimitsBuilder, Trap, TypedFunc, ValType};

pub const MAX_MODULE_BYTES: usize = 1024 * 1024;
pub const MAX_MEMORY: usize = 64 * 1024 * 1024;
/// Initialisation du module (fonction `start`, `traversal_buffer`).
const SETUP_FUEL: u64 = 200_000_000;
/// Journal gardé par partie (`traversal.log`), en mode verbeux seulement.
const MAX_LOG: usize = 64 * 1024;

/// Pourquoi un bot perd la partie sur tapis vert.
#[derive(Clone, Debug)]
pub enum Fault {
    /// Module invalide, imports ou exports non conformes.
    Setup(String),
    /// Fuel épuisé pendant le coup.
    OutOfFuel,
    /// Plantage (trap) : accès mémoire hors limites, `unreachable`, panique…
    Crash(String),
    /// Indice de coup hors de la liste.
    Illegal(i32),
}

impl Fault {
    pub fn code(&self) -> &'static str {
        match self {
            Fault::Setup(_) => "setup",
            Fault::OutOfFuel => "fuel",
            Fault::Crash(_) => "crash",
            Fault::Illegal(_) => "illegal",
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::Setup(m) => write!(f, "module refusé : {m}"),
            Fault::OutOfFuel => write!(f, "fuel épuisé"),
            Fault::Crash(m) => write!(f, "plantage : {m}"),
            Fault::Illegal(i) => write!(f, "coup inexistant (indice {i})"),
        }
    }
}

pub fn engine() -> Engine {
    let mut c = Config::new();
    c.consume_fuel(true);
    c.cranelift_nan_canonicalization(true);
    c.wasm_relaxed_simd(false);
    // Pas de threads : la fonctionnalité n'est pas compilée dans wasmtime (Cargo.toml).
    c.wasm_multi_memory(false);
    c.wasm_memory64(false);
    c.max_wasm_stack(1024 * 1024);
    // Pas de memfd pour initialiser la mémoire : interdit par le filtre seccomp (isolate.rs).
    c.memory_init_cow(false);
    Engine::new(&c).expect("configuration wasmtime")
}

/// Compile et vérifie un module : taille, imports (seulement `traversal.*`), exports.
pub fn compile(engine: &Engine, bytes: &[u8]) -> Result<Module, Fault> {
    if bytes.len() > MAX_MODULE_BYTES {
        return Err(Fault::Setup(format!("{} Ko, maximum {} Ko", bytes.len() / 1024, MAX_MODULE_BYTES / 1024)));
    }
    let module = Module::new(engine, bytes).map_err(|e| Fault::Setup(format!("WebAssembly invalide : {e}")))?;
    for imp in module.imports() {
        let ok = match (imp.module(), imp.name(), imp.ty()) {
            ("traversal", "fuel_left", ExternType::Func(f)) => {
                f.params().len() == 0 && f.results().map(|r| matches!(r, ValType::I64)).eq([true])
            }
            ("traversal", "log", ExternType::Func(f)) => {
                f.params().map(|p| matches!(p, ValType::I32)).eq([true, true]) && f.results().len() == 0
            }
            _ => false,
        };
        if !ok {
            return Err(Fault::Setup(format!(
                "import interdit : {}.{} (seuls traversal.fuel_left et traversal.log existent)",
                imp.module(),
                imp.name()
            )));
        }
    }
    let export = |name: &str| module.get_export(name);
    match export("memory") {
        Some(ExternType::Memory(_)) => {}
        _ => return Err(Fault::Setup("export « memory » manquant".into())),
    }
    let func_ok = |name: &str, params: &[bool]| match export(name) {
        Some(ExternType::Func(f)) => {
            f.params().map(|p| matches!(p, ValType::I32)).eq(params.iter().copied())
                && f.results().map(|r| matches!(r, ValType::I32)).eq([true])
        }
        _ => false,
    };
    if !func_ok("traversal_buffer", &[]) {
        return Err(Fault::Setup("export « traversal_buffer: () -> i32 » manquant".into()));
    }
    if !func_ok("traversal_move", &[true]) {
        return Err(Fault::Setup("export « traversal_move: (i32) -> i32 » manquant".into()));
    }
    Ok(module)
}

struct Data {
    limits: StoreLimits,
    log: Option<String>,
}

/// Un bot instancié pour une partie (sa mémoire persiste d'un coup à l'autre).
pub struct Bot {
    store: Store<Data>,
    memory: Memory,
    buffer: usize,
    choose: TypedFunc<i32, i32>,
    /// Fuel consommé au total (statistiques).
    pub fuel_used: u64,
    pub calls: u64,
}

fn fault_of(e: wasmtime::Error) -> Fault {
    match e.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => Fault::OutOfFuel,
        Some(t) => Fault::Crash(t.to_string()),
        None => Fault::Crash(e.to_string()),
    }
}

impl Bot {
    pub fn new(engine: &Engine, module: &Module, verbose: bool) -> Result<Bot, Fault> {
        let limits = StoreLimitsBuilder::new().memory_size(MAX_MEMORY).memories(1).tables(4).table_elements(100_000)
            .instances(1).build();
        let mut store = Store::new(engine, Data { limits, log: verbose.then(String::new) });
        store.limiter(|d| &mut d.limits);
        store.set_fuel(SETUP_FUEL).expect("fuel activé");

        let mut linker: Linker<Data> = Linker::new(engine);
        linker
            .func_wrap("traversal", "fuel_left", |caller: Caller<'_, Data>| caller.get_fuel().unwrap_or(0) as i64)
            .and_then(|l| {
                l.func_wrap("traversal", "log", |mut caller: Caller<'_, Data>, ptr: i32, len: i32| {
                    if caller.data().log.is_none() {
                        return;
                    }
                    let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else { return };
                    let (start, len) = (ptr as u32 as usize, (len as u32 as usize).min(MAX_LOG));
                    let msg = mem.data(&caller).get(start..start.saturating_add(len)).map(|b| String::from_utf8_lossy(b).into_owned());
                    if let (Some(msg), Some(log)) = (msg, caller.data_mut().log.as_mut()) {
                        if log.len() + msg.len() < MAX_LOG {
                            log.push_str(&msg);
                            log.push('\n');
                        }
                    }
                })
            })
            .expect("fonctions de l'hôte");

        let instance: Instance = linker.instantiate(&mut store, module).map_err(|e| match fault_of(e) {
            Fault::Crash(m) => Fault::Setup(m),
            f => f,
        })?;
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| Fault::Setup("mémoire absente".into()))?;
        let buffer_fn = instance.get_typed_func::<(), i32>(&mut store, "traversal_buffer").map_err(|e| Fault::Setup(e.to_string()))?;
        let choose = instance.get_typed_func::<i32, i32>(&mut store, "traversal_move").map_err(|e| Fault::Setup(e.to_string()))?;
        let buffer = buffer_fn.call(&mut store, ()).map_err(fault_of)? as u32 as usize;
        if buffer.checked_add(BUFFER_SIZE).map_or(true, |end| end > memory.data_size(&store)) {
            return Err(Fault::Setup(format!("traversal_buffer doit désigner {BUFFER_SIZE} octets dans la mémoire")));
        }
        Ok(Bot { store, memory, buffer, choose, fuel_used: 0, calls: 0 })
    }

    /// Indice du coup choisi dans `moves`, avec au plus `fuel` unités de calcul.
    pub fn choose(&mut self, state: &State, moves: &[Move], seed: u64, fuel: u64) -> Result<usize, Fault> {
        let input = wire::encode(state, moves, seed, fuel);
        self.memory
            .write(&mut self.store, self.buffer, &input)
            .map_err(|_| Fault::Crash("tampon hors de la mémoire".into()))?;
        self.store.set_fuel(fuel).expect("fuel activé");
        let res = self.choose.call(&mut self.store, input.len() as i32);
        self.fuel_used += fuel - self.store.get_fuel().unwrap_or(0);
        self.calls += 1;
        let idx = res.map_err(fault_of)?;
        if idx < 0 || idx as usize >= moves.len() {
            return Err(Fault::Illegal(idx));
        }
        Ok(idx as usize)
    }

    pub fn take_log(&mut self) -> String {
        self.store.data_mut().log.as_mut().map(std::mem::take).unwrap_or_default()
    }
}
