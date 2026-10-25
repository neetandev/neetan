//! The fixtures of the tests of the manager: a root folder with game files, the sample games of
//! the screenshots and the count of a steady frame.

use common::temporary::TemporaryFolder;
use machine_factory::config::systems::{System, compiled_systems};

use super::{Manager, entries::TableEntry};
use crate::{
    config::{LaunchRequest, Locations, ManagerStart, SETTINGS_FILE_NAME},
    gui_test_support::{GuiHarness, PassCounts},
};

/// A root folder with a settings file and game files. Dropping the value removes it.
pub(super) struct TestRoot {
    /// The root folder.
    pub(super) root: TemporaryFolder,
}

impl TestRoot {
    /// Creates a root folder with an empty settings file.
    pub(super) fn new(name: &str) -> Self {
        let root = TemporaryFolder::new(&format!("manager_test_{name}"));
        root.write_file(SETTINGS_FILE_NAME, b"");
        Self { root }
    }

    /// Writes the game file `identity` below the config folder with `contents`.
    pub(super) fn game(&self, identity: &str, contents: &str) {
        let path = self.root.path().join("config").join(identity);
        std::fs::create_dir_all(path.parent().expect("game files lie in a folder"))
            .expect("folder is created");
        std::fs::write(path, contents).expect("game file is written");
    }

    /// Removes the game file `identity`.
    pub(super) fn remove(&self, identity: &str) {
        std::fs::remove_file(self.root.path().join("config").join(identity))
            .expect("game file is removed");
    }

    /// The manager of the root.
    pub(super) fn manager(&self) -> Manager {
        let locations = Locations {
            current_directory: self.root.path().to_path_buf(),
            executable_directory: None,
            pref_path: None,
        };
        Manager::new(ManagerStart {
            request: LaunchRequest::default(),
            locations,
        })
        .expect("manager starts")
    }
}

/// The first compiled system.
pub(super) fn first_system() -> &'static System {
    compiled_systems().next().expect("one system is compiled")
}

/// The titles of the entries of the game table of `manager`, with `|` for the caption.
pub(super) fn titles(manager: &Manager) -> Vec<String> {
    let rows = manager.rows();
    manager
        .entries
        .iter()
        .map(|entry| match entry {
            TableEntry::Game(index) => rows[*index].title.clone(),
            TableEntry::Broken(index) => rows[*index].file_name.clone(),
            TableEntry::Caption => "|".to_owned(),
        })
        .collect()
}

/// Writes the games `names` with their names as titles into the first system of `test`.
pub(super) fn games(test: &TestRoot, names: &[&str]) {
    let system = first_system();
    for name in names {
        test.game(
            &format!("{}/{name}.conf", system.name),
            &format!("meta.title = {name}\n"),
        );
    }
}

/// The sample games of the screenshots: the system, the file stem, the
/// title, the original title, the developer and the year.
pub(super) const SAMPLE_GAMES: [(&str, &str, &str, &str, &str, &str); 22] = [
    (
        "pc98",
        "brandish",
        "Brandish",
        "ブランディッシュ",
        "Nihon Falcom",
        "1991",
    ),
    ("pc98", "dokyusei", "Dokyusei", "同級生", "Elf", "1992"),
    (
        "pc98",
        "dragon-slayer",
        "Dragon Slayer: The Legend of Heroes",
        "ドラゴンスレイヤー英雄伝説",
        "Nihon Falcom",
        "1989",
    ),
    ("pc98", "edge", "Edge", "エッジ", "TGL", "1993"),
    (
        "pc98",
        "geneitoshi",
        "Gen'ei Toshi: Illusion City",
        "幻影都市",
        "Microcabin",
        "1991",
    ),
    (
        "pc98",
        "lord-monarch",
        "Lord Monarch",
        "ロードモナーク",
        "Nihon Falcom",
        "1991",
    ),
    (
        "pc98",
        "policenauts",
        "Policenauts",
        "ポリスノーツ",
        "Konami",
        "1994",
    ),
    (
        "pc98",
        "princess-maker-2",
        "Princess Maker 2",
        "プリンセスメーカー2",
        "Gainax",
        "1993",
    ),
    (
        "pc98",
        "puyo-puyo",
        "Puyo Puyo",
        "ぷよぷよ",
        "Compile",
        "1993",
    ),
    (
        "pc98",
        "sangokushi-2",
        "Romance of the Three Kingdoms II",
        "三國志II",
        "Koei",
        "1989",
    ),
    ("pc98", "rusty", "Rusty", "ラスティ", "C-Lab", "1993"),
    (
        "pc98",
        "tokimeki",
        "Tokimeki Memorial",
        "ときめきメモリアル",
        "Konami",
        "1995",
    ),
    (
        "pc98",
        "touhou-fuumaroku",
        "Touhou Fuumaroku: Story of Eastern Wonderland",
        "東方封魔録",
        "Amusement Makers",
        "1997",
    ),
    (
        "pc98",
        "touhou-gensoukyou",
        "Touhou Gensoukyou: Lotus Land Story",
        "東方幻想郷",
        "Amusement Makers",
        "1998",
    ),
    (
        "pc98",
        "touhou-reiiden",
        "Touhou Reiiden: Highly Responsive to Prayers",
        "東方靈異伝",
        "Amusement Makers",
        "1997",
    ),
    (
        "pc98",
        "touhou-yumejikuu",
        "Touhou Yumejikuu: Phantasmagoria of Dim.Dream",
        "東方夢時空",
        "Amusement Makers",
        "1997",
    ),
    (
        "pc98",
        "ys-2",
        "Ys II: Ancient Ys Vanished The Final Chapter",
        "イースII",
        "Nihon Falcom",
        "1988",
    ),
    (
        "pc98",
        "yu-no",
        "YU-NO: A Girl Who Chants Love at the Bound of this World",
        "この世の果てで恋を唄う少女YU-NO",
        "Elf",
        "1996",
    ),
    (
        "x68k",
        "akumajou-dracula",
        "Akumajou Dracula",
        "悪魔城ドラキュラ",
        "Konami",
        "1993",
    ),
    (
        "x68k",
        "final-fight",
        "Final Fight",
        "ファイナルファイト",
        "Capcom",
        "1992",
    ),
    (
        "x68k",
        "gradius-2",
        "Gradius II",
        "グラディウスII",
        "Konami",
        "1992",
    ),
    ("pc88", "ys-3", "", "", "", ""),
];

/// Writes the sample games and two files with errors into `test`.
pub(super) fn sample_root(test: &TestRoot) {
    for (system, stem, title, original, developer, year) in SAMPLE_GAMES {
        let mut contents = String::new();
        for (key, value) in [
            ("meta.title", title),
            ("meta.title-original", original),
            ("meta.developer", developer),
            ("meta.year", year),
        ] {
            if !value.is_empty() {
                contents.push_str(&format!("{key} = {value}\n"));
            }
        }
        test.game(&format!("{system}/{stem}.conf"), &contents);
    }
    test.game(
        "x68k/nemesis-90.conf",
        "meta.title = Nemesis '90\nmachine = X68030\n",
    );
    test.game(
        "x68k/cyberblock-metal-orange.conf",
        "meta.title = Cyber Block\nmachine = PC9801RA\n",
    );
}

/// Selects the system `name` in `manager`.
pub(super) fn select(manager: &mut Manager, name: &str) {
    let index = manager
        .systems
        .iter()
        .position(|entry| entry.system.name == name)
        .expect("the system is compiled");
    manager.select_system(index);
}

/// Draws a frame of `manager` and counts the steady frame after it, the second pass with an
/// unchanged state.
pub(super) fn steady_frame(harness: &mut GuiHarness, manager: &mut Manager) -> PassCounts {
    harness.frame(|context| manager.show(context));
    harness.counted_frame(|context| manager.show(context)).0
}
