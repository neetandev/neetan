use crate::{Error, Reader, expand::Form};

/// Reads one complete source unit and converts its datums into expander forms.
pub(crate) fn read_forms(reader: &mut Reader) -> Result<Vec<Form>, Error> {
    let mut datums = Vec::new();
    while let Some(datum) = reader.read_next()? {
        datums.push(datum);
    }
    crate::expand::forms(&datums)
}
