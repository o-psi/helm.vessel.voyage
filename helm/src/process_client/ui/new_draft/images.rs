use super::*;

impl App {
    pub(in crate::process_client::ui) fn new_draft_images(
        &self,
        id: Uuid,
    ) -> Result<&[super::super::attachments::Image]> {
        Ok(&self
            .new_drafts
            .get(&id)
            .context("Draft unavailable")?
            .saved
            .images)
    }

    pub(in crate::process_client::ui) fn ensure_draft_images_editable(
        &self,
        id: Uuid,
    ) -> Result<()> {
        let draft = self.new_drafts.get(&id).context("Draft unavailable")?;
        anyhow::ensure!(
            !draft.busy && draft.saved.start.is_none(),
            "First send pending; text and attachments are frozen"
        );
        Ok(())
    }

    pub(in crate::process_client::ui) fn set_new_draft_images(
        &mut self,
        id: Uuid,
        images: Vec<super::super::attachments::Image>,
    ) -> Result<()> {
        self.ensure_draft_images_editable(id)?;
        let draft = self.new_drafts.get_mut(&id).context("Draft unavailable")?;
        let mut saved = draft.saved.clone();
        saved.images = images;
        saved.text = draft.composer.text.clone();
        storage::save(&saved)?;
        draft.saved = saved;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_legacy_saved_draft_omits_empty_images() {
        let saved = Saved {
            id: Uuid::from_u128(1),
            route: "local".into(),
            workspace: "/tmp".into(),
            config: None,
            explicit: Default::default(),
            selection: None,
            confirmation: None,
            text: " keep\n".into(),
            images: Vec::new(),
            start: None,
            start_attempted: false,
            process: None,
            turn: Uuid::from_u128(2),
            submit: None,
            attempted: false,
            finished: false,
            receipt: None,
        };
        let value = serde_json::to_value(&saved).unwrap();
        assert!(value.get("images").is_none());
        let recovered: Saved = serde_json::from_value(value).unwrap();
        assert!(recovered.images.is_empty());
        assert_eq!(recovered.text, saved.text);
    }
}
