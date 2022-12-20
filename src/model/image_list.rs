use std::borrow::Borrow;
use std::cell::Cell;
use std::cell::RefCell;

use gtk::gio;
use gtk::glib;
use gtk::glib::clone;
use gtk::glib::subclass::Signal;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use indexmap::map::Entry;
use indexmap::IndexMap;
use once_cell::sync::Lazy;
use once_cell::unsync::OnceCell;

use crate::model;
use crate::model::SelectableListExt;
use crate::podman;
use crate::utils;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct ImageList {
        pub(super) client: glib::WeakRef<model::Client>,
        pub(super) list: RefCell<IndexMap<String, model::Image>>,
        pub(super) listing: Cell<bool>,
        pub(super) initialized: OnceCell<()>,
        pub(super) selection_mode: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImageList {
        const NAME: &'static str = "ImageList";
        type Type = super::ImageList;
        type Interfaces = (gio::ListModel, model::SelectableList);
    }

    impl ObjectImpl for ImageList {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder("image-added")
                    .param_types([model::Image::static_type()])
                    .build()]
            });
            SIGNALS.as_ref()
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<model::Client>("client")
                        .construct_only()
                        .build(),
                    glib::ParamSpecUInt::builder("len").read_only().build(),
                    glib::ParamSpecBoolean::builder("listing")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("initialized")
                        .read_only()
                        .build(),
                    glib::ParamSpecUInt::builder("intermediates")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("selection-mode").build(),
                    glib::ParamSpecUInt::builder("num-selected")
                        .read_only()
                        .build(),
                ]
            });
            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "client" => self.client.set(value.get().unwrap()),
                "selection-mode" => self.selection_mode.set(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = &*self.obj();
            match pspec.name() {
                "client" => obj.client().to_value(),
                "len" => obj.len().to_value(),
                "listing" => obj.listing().to_value(),
                "initialized" => obj.is_initialized().to_value(),
                "intermediates" => obj.intermediates().to_value(),
                "selection-mode" => self.selection_mode.get().to_value(),
                "num-selected" => obj.num_selected().to_value(),
                _ => unimplemented!(),
            }
        }
        fn constructed(&self) {
            self.parent_constructed();
            let obj = &*self.obj();
            model::SelectableList::bootstrap(obj.upcast_ref());
            obj.connect_items_changed(|self_, _, _, _| self_.notify("len"));
        }
    }

    impl ListModelImpl for ImageList {
        fn item_type(&self) -> glib::Type {
            model::Image::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get_index(position as usize)
                .map(|(_, obj)| obj.upcast_ref())
                .cloned()
        }
    }
}

glib::wrapper! {
    pub(crate) struct ImageList(ObjectSubclass<imp::ImageList>)
        @implements gio::ListModel, model::SelectableList;
}

impl From<&model::Client> for ImageList {
    fn from(client: &model::Client) -> Self {
        glib::Object::builder().property("client", client).build()
    }
}

impl ImageList {
    pub(crate) fn client(&self) -> Option<model::Client> {
        self.imp().client.upgrade()
    }

    pub(crate) fn len(&self) -> u32 {
        self.n_items()
    }

    pub(crate) fn listing(&self) -> bool {
        self.imp().listing.get()
    }

    fn set_listing(&self, value: bool) {
        if self.listing() == value {
            return;
        }
        self.imp().listing.set(value);
        self.notify("listing");
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.imp().initialized.get().is_some()
    }

    fn set_as_initialized(&self) {
        if self.is_initialized() {
            return;
        }
        self.imp().initialized.set(()).unwrap();
        self.notify("initialized");
    }

    pub(crate) fn intermediates(&self) -> u32 {
        self.imp()
            .list
            .borrow()
            .values()
            .filter(|image| image.repo_tags().n_items() == 0)
            .count() as u32
    }

    pub(crate) fn total_size(&self) -> u64 {
        self.imp()
            .list
            .borrow()
            .values()
            .map(model::Image::size)
            .sum()
    }

    pub(crate) fn unused_size(&self) -> u64 {
        self.imp()
            .list
            .borrow()
            .values()
            .filter(|image| image.repo_tags().n_items() == 0)
            .map(model::Image::size)
            .sum()
    }

    pub(crate) fn get_image<Q: Borrow<str> + ?Sized>(&self, id: &Q) -> Option<model::Image> {
        self.imp().list.borrow().get(id.borrow()).cloned()
    }

    pub(crate) fn remove_image(&self, id: &str) {
        let mut list = self.imp().list.borrow_mut();
        if let Some((idx, _, image)) = list.shift_remove_full(id) {
            image.emit_deleted();
            drop(list);
            self.items_changed(idx as u32, 1, 0);
        }
    }

    pub(crate) fn refresh<F>(&self, err_op: F)
    where
        F: FnOnce(super::RefreshError) + Clone + 'static,
    {
        self.set_listing(true);
        utils::do_async(
            {
                let podman = self.client().unwrap().podman().clone();
                async move {
                    podman
                        .images()
                        .list(&podman::opts::ImageListOpts::builder().all(true).build())
                        .await
                }
            },
            clone!(@weak self as obj => move |result| {
                match result {
                    Ok(summaries) => {
                        let to_remove = obj
                            .imp()
                            .list
                            .borrow()
                            .keys()
                            .filter(|id| {
                                !summaries
                                    .iter()
                                    .any(|summary| summary.id.as_ref() == Some(id))
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        to_remove.iter().for_each(|id| {
                            obj.remove_image(id);
                        });

                        summaries.iter().for_each(|summary| {
                            let index = obj.len();

                            let mut list = obj.imp().list.borrow_mut();

                            match list.entry(summary.id.as_ref().unwrap().to_owned()) {
                                Entry::Vacant(e) => {
                                    let image = model::Image::new(&obj, summary);
                                    e.insert(image.clone());

                                    drop(list);

                                    obj.items_changed(index, 0, 1);
                                    obj.image_added(&image);
                                }
                                Entry::Occupied(e) => {
                                    let image = e.get().to_owned();
                                    drop(list);
                                    image.update(summary);
                                }
                            }
                        });
                    }
                    Err(e) => {
                        log::error!("Error on retrieving images: {}", e);
                        err_op(super::RefreshError);
                    }
                }
                obj.set_listing(false);
                obj.set_as_initialized();
            }),
        );
    }

    fn tag(&self, id: &str, tag: &str) {
        if let Some(image) = self.imp().list.borrow().get(id) {
            let repo_tags = image.repo_tags();
            let repo_tags_len = repo_tags.len();
            repo_tags.add(model::RepoTag::new(repo_tags, tag));

            if repo_tags_len == 0 {
                self.notify("intermediates");
            }
        }
    }

    fn untag(&self, id: &str, tag: &str) {
        if let Some(image) = self.imp().list.borrow().get(id) {
            let repo_tags = image.repo_tags();
            repo_tags.remove(tag);

            if repo_tags.len() == 0 {
                self.notify("intermediates");
            }
        }
    }

    pub(crate) fn handle_event<F>(&self, event: podman::models::Event, err_op: F)
    where
        F: FnOnce(super::RefreshError) + Clone + 'static,
    {
        match event.action.as_str() {
            "tag" => self.tag(
                &event.actor.id,
                &format!("localhost/{}", event.actor.attributes.get("name").unwrap()),
            ),
            "untag" => self.untag(&event.actor.id, event.actor.attributes.get("name").unwrap()),
            "remove" => self.remove_image(&event.actor.id),
            "build" | "pull" => self.refresh(err_op),
            other => log::warn!("Unknown action: {other}"),
        }
    }

    fn image_added(&self, image: &model::Image) {
        image.connect_notify_local(
            Some("repo-tags"),
            clone!(@weak self as obj => move |_, _| obj.notify("intermediates")),
        );
        self.emit_by_name::<()>("image-added", &[image]);
    }

    pub(crate) fn connect_image_added<F: Fn(&Self, &model::Image) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("image-added", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let image = values[1].get::<model::Image>().unwrap();
            f(&obj, &image);

            None
        })
    }
}
