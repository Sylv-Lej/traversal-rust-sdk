//! Le cloisonnement par processus (isolate.rs), avec le vrai binaire.

use std::path::PathBuf;
use std::process::Command;

const ARENA: &str = env!("CARGO_BIN_EXE_traversal-arena");

/// Bot écrit à la main (texte WAT, accepté par wasmtime).
fn bot(body: &str) -> String {
    format!(
        r#"(module (memory (export "memory") 1)
             (func (export "traversal_buffer") (result i32) (i32.const 1024))
             (func (export "traversal_move") (param $len i32) (result i32) {body}))"#
    )
}

fn dir_with_galet(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("arena-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("1.wasm"), bot("(i32.const 0)")).unwrap();
    dir
}

fn arena(args: &[&str], house: &PathBuf) -> (bool, String) {
    let out = Command::new(ARENA).args(args).arg("--house").arg(house).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
#[cfg(target_os = "linux")]
fn child_process_is_locked_down() {
    let house = dir_with_galet("selftest");
    let (ok, out) = arena(&["selftest"], &house);
    assert!(ok, "{out}");
}

#[test]
fn check_runs_in_the_child_process() {
    let house = dir_with_galet("check");
    let good = house.join("good.wasm");
    std::fs::write(&good, bot("(i32.const 0)")).unwrap();
    let (ok, out) = arena(&["check", good.to_str().unwrap()], &house);
    assert!(ok, "{out}");
    let bad = house.join("bad.wasm");
    std::fs::write(&bad, bot("(loop $l (br $l)) (i32.const 0)")).unwrap();
    let (ok, out) = arena(&["check", bad.to_str().unwrap()], &house);
    assert!(!ok && out.contains("fuel épuisé"), "{out}");
}
