use avc::engine::{Edit, Engine, Limits};
use avc::{Entry, Id, Node, Object};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn records(corpus: &Value) -> Result<Vec<(Id, Vec<u8>)>, String> {
    corpus["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| {
            Ok((
                record["id"]
                    .as_str()
                    .unwrap()
                    .parse::<Id>()
                    .map_err(|e| e.to_string())?,
                unhex(record["payload"].as_str().unwrap()),
            ))
        })
        .collect()
}
fn check(corpus: &Value) -> Result<(), String> {
    let records = records(corpus)?;
    let mut seen = BTreeSet::new();
    for (id, _) in &records {
        if !seen.insert(id) {
            return Err("duplicate corpus object".into());
        }
    }
    for (id, payload) in &records {
        Object::decode(*id, payload).map_err(|e| e.to_string())?;
    }
    for root in corpus["roots"].as_array().unwrap() {
        let root: Id = root
            .as_str()
            .unwrap()
            .parse::<Id>()
            .map_err(|e| e.to_string())?;
        if root.kind == 1 {
            continue;
        } // Blob is a codec entry point, not protected source.
        Engine::new()
            .import(records.clone(), root, Limits::default())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[test]
fn independent_python_vectors_and_negative_corpus() {
    let corpus: Value = serde_json::from_str(include_str!("../reference/vectors.json")).unwrap();
    check(&corpus).unwrap();
    for negative in corpus["negative"].as_array().unwrap() {
        assert!(check(negative).is_err(), "accepted {}", negative["name"]);
    }
}

fn closure(engine: &Engine, root: Id) -> Vec<(Id, Vec<u8>)> {
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let object = engine.object(id).unwrap();
        let mut entries = Vec::new();
        match object {
            Object::Blob(_) => {}
            Object::Map(Node::Leaf(bindings)) => entries.extend(bindings.iter().map(|(_, e)| e)),
            Object::Map(Node::Branch {
                terminal, children, ..
            }) => {
                entries.extend(terminal);
                pending.extend(children.values());
            }
            Object::Conflict(c) => {
                entries.extend(c.positive.iter().chain(&c.negative).map(|(e, _)| e))
            }
            Object::Revision(r) => {
                pending.push(r.root);
                pending.extend(&r.parents);
            }
        }
        for entry in entries {
            match entry {
                Entry::Absent => {}
                Entry::File { blob, .. }
                | Entry::Symlink(blob)
                | Entry::Directory(blob)
                | Entry::Conflict(blob) => pending.push(*blob),
            }
        }
        output.push((id, object.encode().unwrap()));
    }
    output
}
/// Explicit opt-in: CI enables this and runs an independent process. Ordinary
/// offline cargo tests still exercise the committed independent vectors above.
#[test]
fn rust_generated_roots_pass_independent_decoder() {
    if std::env::var_os("AVC_REFERENCE_CHECK").is_none() {
        return;
    }
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(b"new source\r\n".to_vec()).unwrap();
    let value = Entry::File {
        blob,
        executable: true,
    };
    let mut root = empty;
    let mut flat = BTreeMap::new();
    for i in 0..400 {
        let name = format!("prefix-{:04}", (i * 197) % 401).into_bytes();
        root = engine
            .edit(
                root,
                &[Edit {
                    path: vec![name.clone()],
                    expected: Entry::Absent,
                    value: value.clone(),
                }],
            )
            .unwrap();
        flat.insert(name, value.clone());
    }
    let objects: Vec<_> = closure(&engine, root)
        .into_iter()
        .map(|(id, payload)| serde_json::json!({"id":id.to_string(),"payload":hex(&payload)}))
        .collect();
    let corpus = serde_json::json!({"objects":objects,"roots":[root.to_string()]});
    let mut child = std::process::Command::new("uv")
        .args(["run", "reference/check.py", "--verify", "/dev/stdin"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(corpus.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let input = serde_json::json!({"objects":[{"id":blob.to_string(),"payload":hex(b"new source\r\n")}],"bindings":flat.iter().map(|(name,e)|serde_json::json!({"name":hex(name),"entry":hex(&e.encode().unwrap())})).collect::<Vec<_>>()});
    let mut child = std::process::Command::new("uv")
        .args(["run", "reference/check.py", "--map"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let computed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(computed["roots"][0].as_str().unwrap(), root.to_string());
}
