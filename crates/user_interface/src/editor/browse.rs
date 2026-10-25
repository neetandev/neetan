//! The file browser of the editor: the field that the open browser fills,
//! the request that opens the browser for a field, and the chosen path that
//! goes into the field.

use machine_factory::config::settings::{PathKind, ValueKind};

use super::{
    Editor,
    fields::FieldValue,
    media_list::{MediaBrowse, place_image},
    rows::image_icon,
};
use crate::file_browser::{BrowseNoun, BrowseRequest};

/// The field that the open file browser fills.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum BrowseTarget {
    /// The single path of the field at this index.
    Path(usize),
    /// The path list of the field at this index, as the media list asked.
    List(usize, MediaBrowse),
}

impl Editor {
    /// Puts the value `value` from the file browser into the field of `target`.
    pub(super) fn apply_browsed(&mut self, target: BrowseTarget, value: String) {
        match target {
            BrowseTarget::Path(index) => self.fields[index].value = FieldValue::Text(value),
            BrowseTarget::List(index, browse) => {
                let field = &mut self.fields[index];
                let mut values = field.images().to_vec();
                let place = place_image(&mut values, browse, value);
                field.value = FieldValue::List(values);
                field.list.select(place);
            }
        }
        self.refresh();
    }

    /// The request for the file browser of `target`.
    pub(super) fn browse_request(&self, target: BrowseTarget) -> BrowseRequest {
        let (index, current) = match target {
            BrowseTarget::Path(index) => (index, Some(self.fields[index].text().to_owned())),
            BrowseTarget::List(index, browse) => {
                let field = &self.fields[index];
                let images = field.images();
                let current = match browse {
                    MediaBrowse::Replace(place) => images.get(place).cloned(),
                    MediaBrowse::Add => images.get(field.list.selected).or(images.last()).cloned(),
                };
                (index, current)
            }
        };
        let setting = self.fields[index].source.setting();
        let kind = match setting.map(|setting| setting.kind) {
            Some(ValueKind::Path(kind)) => kind,
            Some(
                ValueKind::PathList
                | ValueKind::Choice(_)
                | ValueKind::OnOff
                | ValueKind::Number(_)
                | ValueKind::Language,
            )
            | None => PathKind::File,
        };
        let types = setting.and_then(|setting| self.file_types(setting));
        let noun = match (kind, types) {
            (PathKind::Folder, _) => BrowseNoun::Folder,
            (PathKind::File, Some(_)) => BrowseNoun::Image,
            (PathKind::File, None) => BrowseNoun::File,
        };
        BrowseRequest {
            noun,
            setting,
            kind,
            types,
            icon: image_icon(types),
            current,
        }
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::find_setting;

    use super::*;
    use crate::{
        editor::test_support::{EditorRoot, fdd1_model, index_of},
        file_browser::english_title,
    };

    /// Verifies the request of a media list and of a single path: the title, the files and the current image.
    #[test]
    fn the_request_follows_the_field() {
        let root = EditorRoot::new("browse_request");
        let fdd1 = find_setting("fdd1").expect("fdd1 is a table key");
        let model = fdd1_model(&root);
        let path = root.game(
            "edge",
            &format!("machine = {model}\nfdd1 = games/a.d88\nfdd1 = games/b.d88\n"),
        );
        let mut editor =
            Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        let index = index_of(&editor, "fdd1");
        let types = fdd1.file_types_for(root.target());
        let request = editor.browse_request(BrowseTarget::List(index, MediaBrowse::Replace(1)));
        assert_eq!(english_title(&request), "Choose image for FDD 1");
        assert_eq!(request.kind, PathKind::File);
        assert_eq!(request.types, types);
        assert_eq!(request.icon, image_icon(types));
        assert_eq!(request.current.as_deref(), Some("games/b.d88"));
        editor.fields[index].list.select(0);
        let request = editor.browse_request(BrowseTarget::List(index, MediaBrowse::Add));
        assert_eq!(request.current.as_deref(), Some("games/a.d88"));
    }

    /// Verifies that a chosen path replaces an image, joins the end of a list and fills a single path.
    #[test]
    fn a_chosen_path_fills_its_field() {
        let root = EditorRoot::new("browse_apply");
        let model = fdd1_model(&root);
        let path = root.game("edge", &format!("machine = {model}\nfdd1 = games/a.d88\n"));
        let mut editor =
            Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        let index = index_of(&editor, "fdd1");
        editor.apply_browsed(
            BrowseTarget::List(index, MediaBrowse::Replace(0)),
            "games/c.d88".to_owned(),
        );
        editor.apply_browsed(
            BrowseTarget::List(index, MediaBrowse::Add),
            "games/d.d88".to_owned(),
        );
        assert_eq!(
            editor.fields[index].values(),
            ["games/c.d88", "games/d.d88"]
        );
        assert_eq!(editor.fields[index].list.selected, 1);
        assert_eq!(editor.missing[index], [true, true]);
        let path_index = editor.fields.iter().position(|field| {
            field
                .source
                .setting()
                .is_some_and(|setting| matches!(setting.kind, ValueKind::Path(_)))
        });
        if let Some(path_index) = path_index {
            editor.apply_browsed(BrowseTarget::Path(path_index), "games/x.img".to_owned());
            assert_eq!(editor.fields[path_index].text(), "games/x.img");
            let request = editor.browse_request(BrowseTarget::Path(path_index));
            assert_eq!(request.current.as_deref(), Some("games/x.img"));
            assert!(english_title(&request).starts_with("Choose "));
        }
    }
}
