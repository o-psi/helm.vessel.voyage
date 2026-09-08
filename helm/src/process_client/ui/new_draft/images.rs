//! Rich draft editing shared with the normal composer paste path.
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
    pub(in crate::process_client::ui) fn new_draft_composer_mut(
        &mut self,
        id: Uuid,
    ) -> Option<&mut composer::Composer> {
        self.new_drafts
            .get_mut(&id)
            .map(|draft| &mut draft.composer)
    }
    pub(in crate::process_client::ui) fn copy_new_draft_images(
        &self,
        id: Uuid,
    ) -> Result<(composer::Composer, Vec<super::super::attachments::Image>)> {
        let draft = self.new_drafts.get(&id).context("Draft unavailable")?;
        Ok((draft.composer.clone(), draft.saved.images.clone()))
    }
    pub(in crate::process_client::ui) fn retain_new_draft_images(
        &mut self,
        id: Uuid,
        composer: composer::Composer,
        images: Vec<super::super::attachments::Image>,
    ) -> Result<()> {
        self.ensure_draft_images_editable(id)?;
        super::super::attachments::content(&composer, &images)?;
        let draft = self.new_drafts.get_mut(&id).context("Draft unavailable")?;
        let mut saved = draft.saved.clone();
        saved.images = images;
        saved.text = composer.text.clone();
        saved.markers = (!saved.images.is_empty()).then(|| composer.markers.clone());
        storage::save(&saved)?;
        draft.saved = saved;
        draft.composer = composer;
        Ok(())
    }
}
