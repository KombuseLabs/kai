//! The kai ticket form as GitHub writes it into an issue's body: each field
//! under a `### <label>` heading, in the order of the form, and
//! `_No response_` for an optional field left empty. The form itself is
//! `.github/ISSUE_TEMPLATE/kai-ticket.yml` in the board repository; its
//! labels must stay those of [`FIELDS`].

use types::Form;

/// The labels of the form's fields, as their headings say them.
const FIELDS: [&str; 5] = ["Instruction", "Repo", "Ref", "Check", "Hidden tests"];

/// What GitHub writes for an optional field left empty.
const NO_RESPONSE: &str = "_No response_";

/// The form's answers in `body`, or `None` when the body has no
/// `### Instruction` heading and so is not the form. A field whose heading
/// is missing, or that was left empty, is `None`.
pub(crate) fn parse(body: &str) -> Option<Form> {
    let mut fields: [Option<Vec<&str>>; 5] = Default::default();
    let mut current = None;
    for line in body.lines() {
        let heading = line
            .strip_prefix("### ")
            .and_then(|heading| FIELDS.iter().position(|field| *field == heading.trim()));
        if let Some(index) = heading {
            fields[index] = Some(Vec::new());
            current = Some(index);
        } else if let Some(lines) = current.and_then(|index: usize| fields[index].as_mut()) {
            lines.push(line);
        }
    }
    fields[0].as_ref()?;
    let [instruction, repo, base_ref, check, hidden_tests] = fields.map(|lines| {
        let text = lines?.join("\n");
        let text = text.trim();
        (!text.is_empty() && text != NO_RESPONSE).then(|| text.to_owned())
    });
    Some(Form {
        instruction,
        repo,
        base_ref,
        check,
        hidden_tests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_is_read_from_under_its_heading() {
        let body = "### Instruction\r\n\r\nCount the widgets.\r\n\r\nThen print them.\r\n\r\n\
                    ### Repo\r\n\r\nexample-org/example-code\r\n\r\n\
                    ### Ref\r\n\r\n0123456789abcdef0123456789abcdef01234567\r\n\r\n\
                    ### Check\r\n\r\ncargo test --locked\r\n\r\n\
                    ### Hidden tests\r\n\r\ntests/hidden/count.rs\r\n";
        assert_eq!(
            parse(body),
            Some(Form {
                instruction: Some("Count the widgets.\n\nThen print them.".to_owned()),
                repo: Some("example-org/example-code".to_owned()),
                base_ref: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
                check: Some("cargo test --locked".to_owned()),
                hidden_tests: Some("tests/hidden/count.rs".to_owned()),
            })
        );
    }

    #[test]
    fn a_field_left_empty_or_missing_is_none() {
        let body = "### Instruction\n\nCount the widgets.\n\n### Check\n\n_No response_\n\n### Hidden tests\n\n";
        let form = parse(body).unwrap();
        assert_eq!(form.instruction.as_deref(), Some("Count the widgets."));
        assert_eq!(form.repo, None);
        assert_eq!(form.check, None);
        assert_eq!(form.hidden_tests, None);
    }

    #[test]
    fn other_headings_stay_part_of_the_field_they_are_in() {
        let body = "### Instruction\n\nCount them.\n\n### Notes\n\nCarefully.\n\n### Repo\n\nexample-org/example-code\n";
        let form = parse(body).unwrap();
        assert_eq!(
            form.instruction.as_deref(),
            Some("Count them.\n\n### Notes\n\nCarefully.")
        );
        assert_eq!(form.repo.as_deref(), Some("example-org/example-code"));
    }

    #[test]
    fn a_body_without_the_instruction_heading_is_not_the_form() {
        assert_eq!(
            parse("Please count the widgets.\n\n### Repo\n\nx/y\n"),
            None
        );
        assert_eq!(parse(""), None);
    }
}
