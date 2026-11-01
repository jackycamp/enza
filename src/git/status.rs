//! Conversion from libgit2 change statuses to filter letters.

use git2::Delta;

pub(super) fn delta_filter_letter(delta: Delta) -> char {
    match delta {
        Delta::Added | Delta::Untracked => 'A',
        Delta::Copied => 'C',
        Delta::Deleted => 'D',
        Delta::Modified => 'M',
        Delta::Renamed => 'R',
        Delta::Typechange => 'T',
        Delta::Conflicted => 'U',
        Delta::Unreadable => 'X',
        _ => 'B',
    }
}
