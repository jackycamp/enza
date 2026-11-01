use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use git2::{Delta, Diff as GitDiff, DiffDelta, Patch};

pub(super) enum SyntheticRead<T> {
    Unavailable,
    Complete(T),
    Cancelled,
}

pub(super) fn for_each_patch(
    diff: &GitDiff<'_>,
    mut includes: impl FnMut(Delta) -> bool,
    is_cancelled: &mut dyn FnMut() -> bool,
    mut visit: impl FnMut(&DiffDelta<'_>, &Patch<'_>, &mut dyn FnMut() -> bool) -> bool,
) -> bool {
    if is_cancelled() {
        return false;
    }

    for (index, delta) in diff.deltas().enumerate() {
        if is_cancelled() {
            return false;
        }
        if !includes(delta.status()) {
            continue;
        }

        let Some(patch) = Patch::from_diff(diff, index).ok().flatten() else {
            continue;
        };
        if is_cancelled() || !visit(&delta, &patch, is_cancelled) {
            return false;
        }
    }

    !is_cancelled()
}

pub(super) fn read_synthetic_added_file<T>(
    workdir: Option<&Path>,
    delta: &DiffDelta<'_>,
    mut value: T,
    mut add_line: impl FnMut(&mut T, usize, &[u8]),
    is_cancelled: &mut dyn FnMut() -> bool,
) -> SyntheticRead<T> {
    if is_cancelled() {
        return SyntheticRead::Cancelled;
    }
    if !matches!(delta.status(), Delta::Added | Delta::Untracked) {
        return SyntheticRead::Unavailable;
    }

    let Some(absolute_path) = workdir
        .zip(delta.new_file().path())
        .map(|(workdir, path)| workdir.join(path))
    else {
        return SyntheticRead::Unavailable;
    };
    let Ok(file) = File::open(absolute_path) else {
        return SyntheticRead::Unavailable;
    };

    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut new_lineno = 0;

    loop {
        if is_cancelled() {
            return SyntheticRead::Cancelled;
        }
        match reader.read_line(&mut line) {
            Ok(0) => return SyntheticRead::Complete(value),
            Ok(_) => {
                new_lineno += 1;
                let text = line
                    .strip_suffix('\n')
                    .map(|text| text.strip_suffix('\r').unwrap_or(text))
                    .unwrap_or(&line);
                add_line(&mut value, new_lineno, text.as_bytes());
                line.clear();
            }
            Err(_) => return SyntheticRead::Unavailable,
        }
    }
}
