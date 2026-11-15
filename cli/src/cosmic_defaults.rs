//! TorqOS's defaults in a person's own COSMIC panel (docs/KNOWN-GAPS.md, the desktop's defaults).
//!
//! COSMIC reads a person's own file `~/.config/cosmic/<component>/v1/<key>` *instead of* the system's for that
//! key (cosmic-config's `Config::get`: the user's value, and only when there is none, the system's): nothing is
//! merged. The panel's `plugins_wings` is one such key, so a person who ever moved an applet has a whole list of
//! their own, and an applet that TorqOS added to its default afterwards (the agent's) never reaches them.
//!
//! This puts each applet TorqOS puts in the default list (the ids `org.torqos.*`) into the person's own list,
//! once. Once, and no more: what was offered is remembered per person (`~/.local/state/torq`), so an applet the
//! person then takes out of the panel stays out. Their own applets and their order are left as they are; the
//! new one goes beside the neighbour it has in the default. The shortcuts do not need this: they are COSMIC's
//! `defaults` key, which cosmic-comp merges under the person's `custom` (modules/desktop.nix).
//!
//! Nothing here is privileged, and it runs as the person, in their own session (`torq-desktop-defaults`). A file
//! it does not understand, or cannot write whole, is left exactly as it was, and the reason is said.

use anyhow::{bail, Context, Result};
use ron::ser::PrettyConfig;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// COSMIC's component and key: the panel's applets, the start wing and the end wing.
const COMPONENT: &str = "com.system76.CosmicPanel.Panel/v1/plugins_wings";
/// What TorqOS's own applets are called.
const OURS: &str = "org.torqos.";
/// The most bytes read of either file: COSMIC's is a few hundred.
const MAX: u64 = 64 * 1024;
/// COSMIC's watchers skip temporary files with this prefix (cosmic-config's `watch`).
const TEMP: &str = ".atomicwrite-torq-defaults";

/// The panel's `plugins_wings`: `Some((start wing, end wing))`.
type Wings = Option<(Vec<String>, Vec<String>)>;

/// What to do with a person's file, and what to remember afterwards.
#[derive(Debug, PartialEq, Eq)]
pub struct Merge {
    /// The person's file as it should now be; none: leave it.
    pub write: Option<String>,
    /// The applets added, in the order added.
    pub added: Vec<String>,
    /// Every applet of the default this person now has been offered (to remember).
    pub offered: BTreeSet<String>,
}

fn parse(text: &str) -> Result<Wings> {
    ron::from_str::<Wings>(text).map_err(|e| anyhow::anyhow!("{e}"))
}

fn ours(wings: &(Vec<String>, Vec<String>)) -> Vec<(bool, usize, &String)> {
    let mut found = vec![];
    for (end, wing) in [(false, &wings.0), (true, &wings.1)] {
        found.extend(wing.iter().enumerate().filter(|(_, id)| id.starts_with(OURS)).map(|(i, id)| (end, i, id)));
    }
    found
}

/// The default's applets of TorqOS's, into the person's panel, those not already offered to them. `person` is
/// their file (none: they have none, and the default reaches them as it is). An error means: leave the file.
pub fn merge(default: &str, person: Option<&str>, offered: &BTreeSet<String>) -> Result<Merge> {
    let Some(default) = parse(default).context("the system's default panel")? else {
        bail!("the system's default panel has no applets");
    };
    let all: BTreeSet<String> = ours(&default).into_iter().map(|(_, _, id)| id.clone()).collect();
    let Some(person) = person else {
        return Ok(Merge { write: None, added: vec![], offered: offered.union(&all).cloned().collect() });
    };
    let Some(mut mine) = parse(person).context("your panel's applets")? else {
        bail!("your panel's file says no applets at all");
    };
    let mut added = vec![];
    let mut now = offered.clone();
    for (end, at, id) in ours(&default) {
        if offered.contains(id) {
            continue;
        }
        now.insert(id.clone());
        if mine.0.contains(id) || mine.1.contains(id) {
            continue;
        }
        let (dwing, wing) = if end { (&default.1, &mut mine.1) } else { (&default.0, &mut mine.0) };
        // After the nearest neighbour before it in the default that the person has too; else before the nearest
        // after it; else at the end of the wing.
        let place = dwing[..at]
            .iter()
            .rev()
            .find_map(|n| wing.iter().position(|w| w == n).map(|p| p + 1))
            .or_else(|| dwing[at + 1..].iter().find_map(|n| wing.iter().position(|w| w == n)))
            .unwrap_or(wing.len());
        wing.insert(place, id.clone());
        added.push(id.clone());
    }
    let write = if added.is_empty() {
        None
    } else {
        Some(ron::ser::to_string_pretty(&Some(mine), PrettyConfig::new()).context("writing your panel's applets")?)
    };
    Ok(Merge { write, added, offered: now })
}

/// The lines of the remembered offers: one id each, comment lines and others ignored.
pub fn offered_read(text: &str) -> BTreeSet<String> {
    text.lines().filter(|l| l.starts_with(OURS) && !l.contains(char::is_whitespace)).map(str::to_string).collect()
}

pub fn offered_text(offered: &BTreeSet<String>) -> String {
    let mut out = String::from("# Applets TorqOS has put in your panel once (torq-desktop-defaults). Taken out, they stay out.\n");
    for id in offered {
        out.push_str(id);
        out.push('\n');
    }
    out
}

/// Where everything is for one person, and where the system's default is.
pub struct Places {
    /// `~/.config/cosmic`
    pub cosmic: PathBuf,
    /// The system's data folders, as `XDG_DATA_DIRS` lists them, each holding `cosmic/<component>/<key>`.
    pub data_dirs: Vec<PathBuf>,
    /// The remembered offers.
    pub state: PathBuf,
}

impl Places {
    pub fn of_home() -> Result<Places> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
        let absolute = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
        let config = absolute("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let state = absolute("XDG_STATE_HOME").unwrap_or_else(|| home.join(".local/state"));
        let dirs = std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        Ok(Places {
            cosmic: config.join("cosmic"),
            data_dirs: std::env::split_paths(&dirs).filter(|p| p.is_absolute()).collect(),
            state: state.join("torq/desktop-defaults"),
        })
    }
}

/// A file read whole and small, or why not. A link is not followed: a person who keeps it elsewhere keeps it.
fn read(path: &Path) -> Result<Option<String>> {
    use std::io::Read;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if !meta.is_file() {
        bail!("{} is not a plain file (a link, perhaps), so it is left as it is", path.display());
    }
    let mut text = String::new();
    std::fs::File::open(path)?.take(MAX + 1).read_to_string(&mut text).with_context(|| format!("reading {}", path.display()))?;
    if text.len() as u64 > MAX {
        bail!("{} is larger than {MAX} bytes", path.display());
    }
    Ok(Some(text))
}

/// Whole or not at all, the way COSMIC writes: to a file beside it, renamed over it.
fn write(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().context("a file has a folder")?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(TEMP);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// What one run did: the lines for the journal.
pub fn run(places: &Places) -> Result<Vec<String>> {
    run_with(places, &mut || ())
}

/// `run`, with something done between reading the person's file and checking it is still what was read (a test
/// plays COSMIC writing it just then).
fn run_with(places: &Places, between: &mut dyn FnMut()) -> Result<Vec<String>> {
    let default = places
        .data_dirs
        .iter()
        .find_map(|d| read(&d.join("cosmic").join(COMPONENT)).transpose())
        .transpose()?;
    let Some(default) = default else { return Ok(vec!["the system has no default panel: nothing to offer".to_string()]) };
    let file = places.cosmic.join(COMPONENT);
    let offered = read(&places.state)?.map(|t| offered_read(&t)).unwrap_or_default();
    // COSMIC may write its file while this reads it: read again before writing, and if it changed, start over.
    for _ in 0..3 {
        let person = read(&file)?;
        let m = merge(&default, person.as_deref(), &offered)?;
        between();
        if let Some(text) = &m.write {
            if read(&file)? != person {
                continue;
            }
            write(&file, text)?;
        }
        if m.offered != offered {
            write(&places.state, &offered_text(&m.offered))?;
        }
        return Ok(m.added.iter().map(|id| format!("added {id} to your panel")).collect());
    }
    bail!("your panel's file kept changing while it was read: tried again at the next login")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// COSMIC's own default, as TorqOS's package makes it (settings/package.nix): the agent right after TorqOS's.
    const DEFAULT: &str = r#"Some((["com.system76.CosmicPanelWorkspacesButton", "com.system76.CosmicPanelAppButton"],["com.system76.CosmicAppletInputSources","com.system76.CosmicAppletA11y", "org.torqos.Applet", "org.torqos.Agent", "com.system76.CosmicAppletStatusArea"]))"#;

    /// The laptop's file, in the style COSMIC writes; no agent, TorqOS's applet and the tunnels' by hand.
    const LAPTOP: &str = r#"Some(([
    "com.system76.CosmicPanelAppButton",
    "com.system76.CosmicAppletWorkspaces",
], [
    "org.torqos.Tunnels",
    "org.torqos.Applet",
    "com.system76.CosmicAppletStatusArea",
]))"#;

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }
    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }
    fn wings(text: &str) -> (Vec<String>, Vec<String>) {
        parse(text).unwrap().unwrap()
    }
    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_person_with_no_file_gets_the_default_as_it_is_and_it_is_remembered_as_offered() {
        let m = merge(DEFAULT, None, &none()).unwrap();
        assert_eq!(m.write, None);
        assert_eq!(m.offered, set(&["org.torqos.Agent", "org.torqos.Applet"]));
    }

    #[test]
    fn the_laptops_file_gains_the_agent_right_after_torqos_and_keeps_everything_else() {
        let m = merge(DEFAULT, Some(LAPTOP), &none()).unwrap();
        assert_eq!(m.added, vec!["org.torqos.Agent"]);
        let w = wings(&m.write.unwrap());
        assert_eq!(w.0, strs(&["com.system76.CosmicPanelAppButton", "com.system76.CosmicAppletWorkspaces"]));
        assert_eq!(w.1, strs(&["org.torqos.Tunnels", "org.torqos.Applet", "org.torqos.Agent", "com.system76.CosmicAppletStatusArea"]));
        assert_eq!(m.offered, set(&["org.torqos.Agent", "org.torqos.Applet"]));
    }

    #[test]
    fn what_the_person_already_has_is_not_added_twice_nor_moved() {
        let have = r#"Some((["a"], ["org.torqos.Agent", "x", "org.torqos.Applet"]))"#;
        let m = merge(DEFAULT, Some(have), &none()).unwrap();
        assert_eq!(m.write, None);
        assert!(m.added.is_empty());
        assert_eq!(m.offered, set(&["org.torqos.Agent", "org.torqos.Applet"]));
    }

    #[test]
    fn an_applet_taken_out_after_it_was_offered_is_not_put_back() {
        let m = merge(DEFAULT, Some(LAPTOP), &set(&["org.torqos.Agent", "org.torqos.Applet"])).unwrap();
        assert_eq!(m.write, None);
        assert!(m.added.is_empty());
    }

    #[test]
    fn only_the_applet_not_yet_offered_is_added() {
        let m = merge(DEFAULT, Some(LAPTOP), &set(&["org.torqos.Applet"])).unwrap();
        assert_eq!(m.added, vec!["org.torqos.Agent"]);
    }

    #[test]
    fn a_neighbour_the_person_lacks_falls_back_to_the_next_one_or_the_wings_end() {
        // Neither neighbour of the agent in the default is theirs: before the next one that is, else the end.
        let lacks = r#"Some((["a"], ["x", "com.system76.CosmicAppletStatusArea"]))"#;
        let w = wings(&merge(DEFAULT, Some(lacks), &set(&["org.torqos.Applet"])).unwrap().write.unwrap());
        assert_eq!(w.1, strs(&["x", "org.torqos.Agent", "com.system76.CosmicAppletStatusArea"]));
        let bare = r#"Some((["a"], ["x"]))"#;
        let w = wings(&merge(DEFAULT, Some(bare), &set(&["org.torqos.Applet"])).unwrap().write.unwrap());
        assert_eq!(w.1, strs(&["x", "org.torqos.Agent"]));
        assert_eq!(w.0, strs(&["a"]));
    }

    #[test]
    fn a_file_that_is_not_a_panel_list_is_refused_whole() {
        for bad in ["", "Some((", "None", "Some((\"a\", \"b\"))", "{ (modifiers: [Super]): Disable }", "Some(([\"a\"],[1]))", "Some(([], [])) trailing"] {
            assert!(merge(DEFAULT, Some(bad), &none()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_default_that_is_not_readable_offers_nothing() {
        assert!(merge("garbage", Some(LAPTOP), &none()).is_err());
        assert!(merge("None", Some(LAPTOP), &none()).is_err());
    }

    #[test]
    fn what_is_written_is_what_cosmic_reads_back() {
        let text = merge(DEFAULT, Some(LAPTOP), &none()).unwrap().write.unwrap();
        assert!(text.starts_with("Some(("), "{text}");
        assert!(parse(&text).is_ok());
    }

    #[test]
    fn the_remembered_offers_round_trip_and_ignore_other_lines() {
        let s = set(&["org.torqos.Agent", "org.torqos.Applet"]);
        assert_eq!(offered_read(&offered_text(&s)), s);
        assert_eq!(offered_read("# x\ncom.system76.Thing\norg.torqos.Agent extra\n  org.torqos.Y\n"), none());
    }

    fn places(d: &Path) -> Places {
        Places { cosmic: d.join("home/cosmic"), data_dirs: vec![d.join("empty"), d.join("sys")], state: d.join("home/state/desktop-defaults") }
    }
    fn put(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn system(d: &Path) {
        put(&d.join("sys/cosmic").join(COMPONENT), DEFAULT);
    }

    #[test]
    fn a_run_adds_once_and_a_person_who_takes_it_out_keeps_it_out() {
        let d = tempfile::tempdir().unwrap();
        let p = places(d.path());
        system(d.path());
        let file = p.cosmic.join(COMPONENT);
        put(&file, LAPTOP);
        assert_eq!(run(&p).unwrap(), vec!["added org.torqos.Agent to your panel"]);
        assert!(wings(&std::fs::read_to_string(&file).unwrap()).1.contains(&"org.torqos.Agent".to_string()));
        assert!(!d.path().join("home/cosmic/com.system76.CosmicPanel.Panel/v1").join(TEMP).exists());
        // Again: nothing.
        assert!(run(&p).unwrap().is_empty());
        // The person takes it out; the next login leaves it out.
        put(&file, LAPTOP);
        assert!(run(&p).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), LAPTOP);
    }

    #[test]
    fn a_run_for_a_person_with_no_file_writes_none_and_remembers() {
        let d = tempfile::tempdir().unwrap();
        let p = places(d.path());
        system(d.path());
        assert!(run(&p).unwrap().is_empty());
        assert!(!p.cosmic.join(COMPONENT).exists());
        // They change the panel later and drop the agent: it is not offered again.
        put(&p.cosmic.join(COMPONENT), LAPTOP);
        assert!(run(&p).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(p.cosmic.join(COMPONENT)).unwrap(), LAPTOP);
    }

    #[test]
    fn a_run_leaves_a_file_it_cannot_read_exactly_as_it_was_and_remembers_nothing() {
        let d = tempfile::tempdir().unwrap();
        let p = places(d.path());
        system(d.path());
        put(&p.cosmic.join(COMPONENT), "Some((oops");
        assert!(run(&p).is_err());
        assert_eq!(std::fs::read_to_string(p.cosmic.join(COMPONENT)).unwrap(), "Some((oops");
        assert!(!p.state.exists());
    }

    #[test]
    fn a_link_is_left_alone() {
        let d = tempfile::tempdir().unwrap();
        let p = places(d.path());
        system(d.path());
        put(&d.path().join("elsewhere"), LAPTOP);
        let file = p.cosmic.join(COMPONENT);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(d.path().join("elsewhere"), &file).unwrap();
        assert!(run(&p).is_err());
        assert_eq!(std::fs::read_to_string(d.path().join("elsewhere")).unwrap(), LAPTOP);
        assert!(std::fs::symlink_metadata(&file).unwrap().file_type().is_symlink());
    }

    #[test]
    fn a_file_cosmic_writes_during_a_run_is_not_overwritten_with_the_older_one() {
        let d = tempfile::tempdir().unwrap();
        let p = places(d.path());
        system(d.path());
        let file = p.cosmic.join(COMPONENT);
        put(&file, LAPTOP);
        let newer = r#"Some((["a"], ["org.torqos.Applet", "mine"]))"#;
        let mut times = 0;
        let said = run_with(&p, &mut || {
            times += 1;
            if times == 1 {
                put(&file, newer);
            }
        })
        .unwrap();
        assert_eq!(said, vec!["added org.torqos.Agent to your panel"]);
        assert_eq!(wings(&std::fs::read_to_string(&file).unwrap()).1, strs(&["org.torqos.Applet", "org.torqos.Agent", "mine"]));
    }

    #[test]
    fn no_system_default_is_nothing_to_do() {
        let d = tempfile::tempdir().unwrap();
        let p = places(d.path());
        put(&p.cosmic.join(COMPONENT), LAPTOP);
        assert_eq!(run(&p).unwrap().len(), 1);
        assert_eq!(std::fs::read_to_string(p.cosmic.join(COMPONENT)).unwrap(), LAPTOP);
        assert!(!p.state.exists());
    }

    #[test]
    fn the_first_default_found_is_the_one_used() {
        let d = tempfile::tempdir().unwrap();
        let mut p = places(d.path());
        system(d.path());
        put(&d.path().join("first/cosmic").join(COMPONENT), r#"Some((["a"],["org.torqos.Only"]))"#);
        p.data_dirs.insert(0, d.path().join("first"));
        put(&p.cosmic.join(COMPONENT), LAPTOP);
        assert_eq!(run(&p).unwrap(), vec!["added org.torqos.Only to your panel"]);
    }
}
