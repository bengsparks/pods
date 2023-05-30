use std::cell::RefCell;

use adw::subclass::prelude::*;
use adw::traits::MessageDialogExt;
use gettextrs::gettext;
use glib::clone;
use glib::closure;
use glib::Properties;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::CompositeTemplate;
use once_cell::unsync::OnceCell as UnsyncOnceCell;

use crate::model;
use crate::widget;

mod imp {
    use super::*;

    #[derive(Debug, Default, Properties, CompositeTemplate)]
    #[properties(wrapper_type = super::ContainerRenameDialog)]
    #[template(file = "container_rename_dialog.ui")]
    pub(crate) struct ContainerRenameDialog {
        pub(super) response: RefCell<Option<String>>,
        pub(super) rename_finished: UnsyncOnceCell<()>,
        #[property(get, set, construct_only, nullable)]
        pub(super) container: glib::WeakRef<model::Container>,
        #[template_child]
        pub(super) entry_row: TemplateChild<widget::RandomNameEntryRow>,
        #[template_child]
        pub(super) error_label_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub(super) error_label: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContainerRenameDialog {
        const NAME: &'static str = "PdsContainerRenameDialog";
        type Type = super::ContainerRenameDialog;
        type ParentType = adw::MessageDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl ContainerRenameDialog {
        #[template_callback]
        fn on_key_pressed(
            &self,
            key: gdk::Key,
            _: u32,
            _: gdk::ModifierType,
            _: &gtk::EventControllerKey,
        ) -> gtk::Inhibit {
            gtk::Inhibit(if key == gdk::Key::Escape {
                self.response.replace(Some("close".to_string()));
                self.obj().close();
                true
            } else {
                false
            })
        }
    }

    impl ObjectImpl for ContainerRenameDialog {
        fn properties() -> &'static [glib::ParamSpec] {
            Self::derived_properties()
        }

        fn set_property(&self, id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            self.derived_set_property(id, value, pspec);
        }

        fn property(&self, id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            self.derived_property(id, pspec)
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = &*self.obj();

            if let Some(container) = obj.container() {
                container.connect_deleted(clone!(@weak obj => move |_| {
                    obj.imp().rename_finished.set(()).unwrap();
                    obj.close();
                }));

                self.entry_row.set_text(&container.name());
                self.entry_row.grab_focus();
            }

            obj.connect_response(None, |obj, response| {
                obj.imp().response.replace(Some(response.to_owned()));
            });

            obj.connect_close_request(|obj| {
                let imp = obj.imp();

                if imp.rename_finished.get().is_some() {
                    return gtk::Inhibit(false);
                }

                match imp.response.take() {
                    Some(response) => {
                        if &response == "close" {
                            return gtk::Inhibit(false);
                        }

                        if let Some(container) = obj.container() {
                            let new_name = imp.entry_row.text().to_string();
                            container.rename(
                                new_name,
                                clone!(@weak obj => move |result| {
                                    let imp = obj.imp();
                                    match result {
                                        Ok(_) => {
                                            imp.rename_finished.set(()).unwrap();
                                            obj.close();
                                        },
                                        Err(e) => {
                                            imp.entry_row.add_css_class("error");
                                            imp.error_label_revealer.set_visible(true);
                                            imp.error_label_revealer.set_reveal_child(true);
                                            imp.error_label.set_text(&e.to_string());
                                        }
                                    }
                                }),
                            );
                        }

                        gtk::Inhibit(true)
                    }
                    None => {
                        glib::idle_add_local_once(clone!(@weak obj => move || {
                            obj.close();
                        }));
                        gtk::Inhibit(true)
                    }
                }
            });

            self.entry_row
                .connect_changed(clone!(@weak obj => move |entry| {
                    let imp = obj.imp();
                    imp.entry_row.remove_css_class("error");
                    imp.error_label_revealer.set_reveal_child(false);
                    obj.set_response_enabled("rename", !entry.text().is_empty());
                }));

            self.error_label_revealer.connect_child_revealed_notify(
                clone!(@weak obj => move |revealer| {
                    if !revealer.reveals_child() {
                        revealer.set_visible(false);
                    }
                }),
            );

            obj.set_heading_use_markup(true);
            Self::Type::this_expression("container")
                .chain_property::<model::Container>("name")
                .chain_closure::<String>(closure!(|_: Self::Type, name: String| {
                    format!(
                        "{}\n<span weight=\"bold\">«{}»</span>",
                        gettext("Rename Container"),
                        name
                    )
                }))
                .bind(obj, "heading", Some(obj));
        }
    }

    impl WidgetImpl for ContainerRenameDialog {}
    impl WindowImpl for ContainerRenameDialog {}
    impl MessageDialogImpl for ContainerRenameDialog {}
}

glib::wrapper! {
    pub(crate) struct ContainerRenameDialog(ObjectSubclass<imp::ContainerRenameDialog>)
        @extends gtk::Widget, gtk::Window, adw::MessageDialog,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl From<&model::Container> for ContainerRenameDialog {
    fn from(container: &model::Container) -> Self {
        glib::Object::builder()
            .property("container", container)
            .build()
    }
}
