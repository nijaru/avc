//! Offline qualification harness, NOT an AVC store, admission service or CLI.
//! Bounded JSON corpus (64 MiB input), in-memory payloads, 256 open references,
//! 30,000 visits per root. These fixture budgets are not canonical format limits.
mod golden;
mod maps;
mod verify;
mod wire;

use maps::{FlatMap, Store};
use serde::Deserialize;
use std::io::{self, Read};
use wire::{Entry, Id, Result, unhex};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusObject {
    id_hex: String,
    payload_hex: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    objects: Vec<CorpusObject>,
    roots: Vec<String>,
}

fn run() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "--golden".into());
    if mode == "--golden" {
        let (vectors, objects) = golden::check()?;
        println!("Rust canonical vectors: {vectors} PASS; objects {objects}");
        return Ok(());
    }
    const INPUT_LIMIT: u64 = 64 * 1024 * 1024;
    let mut bytes = Vec::new();
    io::stdin()
        .take(INPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > INPUT_LIMIT {
        return Err("corpus input budget".into());
    }
    let output = match mode.as_str() {
        "--objects-stdin" => {
            let corpus: Corpus = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let mut store = Store::new();
            for object in corpus.objects {
                let payload = unhex(object.payload_hex.as_bytes())?;
                let id = Id::from_bytes(&unhex(object.id_hex.as_bytes())?)?;
                // Explicit keys permit corrupt-at-address negative cases. Only
                // graph traversal decides which payloads must hash/parse.
                if store.insert(id, payload).is_some() {
                    return Err("duplicate corpus object".into());
                }
            }
            let verdicts: Vec<_> = corpus
                .roots
                .iter()
                .map(|hex| {
                    unhex(hex.as_bytes())
                        .and_then(|b| Id::from_bytes(&b))
                        .and_then(|id| verify::Verifier::check(&store, id))
                        .is_ok()
                })
                .collect();
            serde_json::to_string(&verdicts)
        }
        "--maps-stdin" => {
            let groups: Vec<Vec<String>> =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if groups.len() > 1000 {
                return Err("map corpus group budget".into());
            }
            let mut roots = Vec::new();
            for names in groups {
                if names.len() > 30_000 {
                    return Err("map corpus name budget".into());
                }
                let mut store = Store::new();
                let blob = maps::put(&mut store, 1, b"fixture".to_vec())?;
                let mut values = FlatMap::new();
                for name in names {
                    if values
                        .insert(unhex(name.as_bytes())?, Entry::File(blob, false))
                        .is_some()
                    {
                        return Err("duplicate input name".into());
                    }
                }
                let root = maps::build(&values, &mut store)?;
                verify::Verifier::check(&store, root)?;
                roots.push(root.display());
            }
            serde_json::to_string(&roots)
        }
        "--ids-stdin" => {
            let ids: Vec<String> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let spellings: Vec<_> = ids
                .iter()
                .map(|text| Id::parse(text).map(Id::display).ok())
                .collect();
            serde_json::to_string(&spellings)
        }
        _ => return Err("unknown corpus mode".into()),
    }
    .map_err(|e| e.to_string())?;
    println!("{output}");
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("codec qualification: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::Encode;
    #[test]
    fn construct_and_decode_all_pinned_kinds() {
        assert_eq!(golden::check(), Ok((25, 42)));
    }
    #[test]
    fn official_hash_and_typed_empty_vectors() {
        assert_eq!(
            blake3::hash(b"").to_hex().as_str(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            blake3::hash(b"abc").to_hex().as_str(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
        let leaf = Id::hash(2, &[0; 4]);
        assert_eq!(
            leaf.display(),
            "avc1-b3-k02-3jc2jl6s7rrrbhdwslqnijidn7z73ly43l7yzcpk37rilrouqvoa"
        );
        let state = wire::Object::State(Entry::Directory(leaf))
            .encoded()
            .expect("fixed State encoding");
        assert_eq!(
            Id::hash(4, &state).display(),
            "avc1-b3-k04-vmnwhh4bsvdgr3igkr7ku35zbdkwkuwugwansvrb6vo3wniyjhmq"
        );
    }
    #[test]
    fn typed_spelling_rejects_aliases() {
        let id = Id::hash(2, &[0; 4]);
        assert_eq!(Id::parse(&id.display()), Ok(id));
        assert!(Id::parse(&id.display().to_uppercase()).is_err());
        let mut bad = id.display();
        bad.pop();
        bad.push('b');
        assert!(Id::parse(&bad).is_err());
        assert!(Id::parse("not-an-id").is_err());
    }
    #[test]
    fn stale_address_is_rejected_but_unowned_payload_is_ignored() {
        let mut store = Store::new();
        let leaf = maps::put(&mut store, 2, vec![0; 4]).expect("empty map");
        let state = maps::put(
            &mut store,
            4,
            wire::Object::State(Entry::Directory(leaf))
                .encoded()
                .expect("State encoding"),
        )
        .expect("State");
        let unrelated = Id::hash(1, b"unrelated private bytes");
        store.insert(unrelated, b"corrupt private address".to_vec());
        assert!(verify::Verifier::check(&store, state).is_ok());
        store.insert(leaf, vec![1, 0, 0, 0]);
        assert!(verify::Verifier::check(&store, state).is_err());
    }
}
