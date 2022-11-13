mod build_page;
mod details_page;
mod history_page;
mod local_combo_row;
mod menu_button;
mod pull_page;
mod row;
mod selection_page;

use adw::prelude::MessageDialogExtManual;
use adw::traits::MessageDialogExt;
use gettextrs::gettext;
use gtk::glib;
use gtk::glib::clone;

pub(crate) use self::build_page::BuildPage;
pub(crate) use self::details_page::DetailsPage;
pub(crate) use self::history_page::HistoryPage;
pub(crate) use self::local_combo_row::LocalComboRow;
pub(crate) use self::menu_button::MenuButton;
pub(crate) use self::pull_page::PullPage;
pub(crate) use self::row::Row;
pub(crate) use self::selection_page::SelectionPage;
use crate::model;
use crate::podman;
use crate::utils;
use crate::view;

fn delete_image_show_confirmation(widget: &gtk::Widget, image: Option<model::Image>) {
    if let Some(image) = image {
        let first_container = image.container_list().get(0);

        if image.containers() > 0 || first_container.is_some() {
            let dialog = adw::MessageDialog::builder()
                .heading(&gettext("Confirm Forced Image Deletion"))
                .body_use_markup(true)
                .body(
                    &match first_container.as_ref().map(|c| c.name()) {
                        Some(id) => gettext!(
                            // Translators: The "{}" is a placeholder for the container name.
                            "Image is used by container <b>{}</b>. Deleting the image will also delete all its associated containers.",
                            id
                        ),
                        None => gettext(
                           "Image is used by a container. Deleting the image will also delete all its associated containers.",
                       ),
                    }

                )
                .modal(true)
                .transient_for(&utils::root(widget)).build();

            dialog.add_responses(&[
                ("cancel", &gettext("_Cancel")),
                ("delete", &gettext("_Force Delete")),
            ]);
            dialog.set_default_response(Some("cancel"));
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);

            dialog.run_async(
                None,
                clone!(@weak widget, @weak image => move |_, response| {
                    if response == "delete" {
                        delete_image(&widget, &image);
                    }
                }),
            );
        } else {
            delete_image(widget, &image);
        }
    }
}

fn delete_image(widget: &gtk::Widget, image: &model::Image) {
    image.delete(clone!(@weak widget => move |image, result| {
        if let Err(e) = result {
            utils::show_toast(
                &widget,
                // Translators: The first "{}" is a placeholder for the image id, the second is for an error message.
                &gettext!("Error on deleting image '{}': {}", image.id(), e)
            );
        }
    }));
}

fn show_inspection(overlay: &view::LeafletOverlay, image: Option<model::Image>) {
    if let Some(image) = image {
        let weak_ref = glib::WeakRef::new();
        weak_ref.set(Some(&image));

        overlay.show_details(&view::SourceViewPage::from(view::Entity::Image(weak_ref)));
    }
}

fn pull_latest(overlay: Option<&view::LeafletOverlay>, image: Option<model::Image>) {
    if let Some(image) = image {
        if let Some(action_list) = image
            .image_list()
            .as_ref()
            .and_then(model::ImageList::client)
            .as_ref()
            .map(model::Client::action_list)
        {
            let reference = image.repo_tags().string(0).unwrap();

            let action = action_list.download_image(
                &reference,
                podman::opts::PullOpts::builder()
                    .reference(reference.as_str())
                    .policy(podman::opts::PullPolicy::Newer)
                    .build(),
            );

            if let Some(overlay) = overlay {
                overlay.show_details(&view::ActionPage::from(&action));
            }
        }
    }
}

pub(crate) fn create_container(overlay: &view::LeafletOverlay, image: Option<model::Image>) {
    if let Some(ref image) = image {
        overlay.show_details(&view::ContainerCreationPage::from(image));
    }
}
