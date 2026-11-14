use rtools::plugins::json::core::Draft;
use std::{fs, io, path::PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtools-draft-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn missing_draft_loads_as_none() {
    assert_eq!(Draft::new(&temp_dir("missing")).load().unwrap(), None);
}

#[test]
fn save_creates_directory_and_round_trips() {
    let dir = temp_dir("roundtrip").join("nested");
    let draft = Draft::new(&dir);
    draft.save("{\"城市\": \"胡杨河市\"}").unwrap();
    assert_eq!(
        draft.load().unwrap().as_deref(),
        Some("{\"城市\": \"胡杨河市\"}")
    );
    draft.save("2").unwrap();
    assert_eq!(draft.load().unwrap().as_deref(), Some("2"));
    // Only the draft itself remains; temp files are renamed away.
    let names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["draft.json"]);
}

#[test]
fn invalid_utf8_draft_is_reported() {
    let dir = temp_dir("utf8");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("draft.json"), [0xff, 0xfe, 0x00]).unwrap();
    assert_eq!(
        Draft::new(&dir).load().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn drafts_of_two_views_save_concurrently() {
    let dir = temp_dir("race");
    let writers: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|text| {
            let draft = Draft::new(&dir);
            std::thread::spawn(move || (0..300).try_for_each(|_| draft.save(text)))
        })
        .collect();
    for writer in writers {
        writer.join().unwrap().unwrap();
    }
    let saved = Draft::new(&dir).load().unwrap().unwrap();
    assert!(saved == "first" || saved == "second", "{saved:?}");
    fs::remove_dir_all(&dir).unwrap();
}
