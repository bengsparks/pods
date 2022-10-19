use std::cell::Cell;
use std::cell::RefCell;

use gtk::gio;
use gtk::glib;
use gtk::glib::clone;
use gtk::glib::subclass::Signal;
use gtk::prelude::ParamSpecBuilderExt;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use indexmap::map::Entry;
use indexmap::map::IndexMap;
use once_cell::sync::Lazy;
use once_cell::unsync::OnceCell;

use crate::model;
use crate::model::SelectableListExt;
use crate::podman;
use crate::utils;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct PodList {
        pub(super) client: glib::WeakRef<model::Client>,
        pub(super) list: RefCell<IndexMap<String, model::Pod>>,
        pub(super) listing: Cell<bool>,
        pub(super) initialized: OnceCell<()>,
        pub(super) selection_mode: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PodList {
        const NAME: &'static str = "PodList";
        type Type = super::PodList;
        type Interfaces = (gio::ListModel, model::SelectableList);
    }

    impl ObjectImpl for PodList {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder("pod-added")
                    .param_types([model::Pod::static_type()])
                    .build()]
            });
            SIGNALS.as_ref()
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<model::Client>("client")
                        .flags(glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY)
                        .build(),
                    glib::ParamSpecUInt::builder("len")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecBoolean::builder("listing")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecBoolean::builder("initialized")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecUInt::builder("running")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecBoolean::builder("selection-mode").build(),
                    glib::ParamSpecUInt::builder("num-selected")
                        .flags(glib::ParamFlags::READABLE)
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
            let obj = &*self.instance();
            match pspec.name() {
                "client" => obj.client().to_value(),
                "len" => obj.len().to_value(),
                "listing" => obj.listing().to_value(),
                "initialized" => obj.is_initialized().to_value(),
                "running" => obj.running().to_value(),
                "selection-mode" => self.selection_mode.get().to_value(),
                "num-selected" => obj.num_selected().to_value(),
                _ => unimplemented!(),
            }
        }
        fn constructed(&self) {
            self.parent_constructed();
            let obj = &*self.instance();
            model::SelectableList::bootstrap(obj);
            obj.connect_items_changed(|self_, _, _, _| self_.notify("len"));
        }
    }

    impl ListModelImpl for PodList {
        fn item_type(&self) -> glib::Type {
            model::Pod::static_type()
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
    pub(crate) struct PodList(ObjectSubclass<imp::PodList>)
        @implements gio::ListModel, model::SelectableList;
}

impl From<Option<&model::Client>> for PodList {
    fn from(client: Option<&model::Client>) -> Self {
        glib::Object::new::<Self>(&[("client", &client)])
    }
}

impl PodList {
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

    pub(crate) fn running(&self) -> u32 {
        self.imp()
            .list
            .borrow()
            .values()
            .filter(|pod| pod.status() == model::PodStatus::Running)
            .count() as u32
    }

    pub(crate) fn get_pod(&self, id: &str) -> Option<model::Pod> {
        self.imp().list.borrow().get(id).cloned()
    }

    pub(crate) fn remove_pod(&self, id: &str) {
        let mut list = self.imp().list.borrow_mut();
        if let Some((idx, _, pod)) = list.shift_remove_full(id) {
            pod.emit_deleted();
            drop(list);
            self.items_changed(idx as u32, 1, 0);
        }
    }

    pub(crate) fn refresh<F>(&self, id: Option<String>, err_op: F)
    where
        F: FnOnce(super::RefreshError) + Clone + 'static,
    {
        self.set_listing(true);
        utils::do_async(
            {
                let podman = self.client().unwrap().podman().clone();
                let id = id.clone();
                async move {
                    podman
                        .pods()
                        .list(
                            &podman::opts::PodListOpts::builder()
                                .filter(
                                    id.map(podman::Id::from)
                                        .map(podman::opts::PodListFilter::Id),
                                )
                                .build(),
                        )
                        .await
                }
            },
            clone!(@weak self as obj => move |result| {
                match result {
                    Ok(list_pods) => {
                        if id.is_none() {
                            let to_remove = obj
                                .imp()
                                .list
                                .borrow()
                                .keys()
                                .filter(|id| {
                                    !list_pods
                                        .iter()
                                        .any(|list_pod| list_pod.id.as_ref() == Some(id))
                                })
                                .cloned()
                                .collect::<Vec<_>>();
                            to_remove.iter().for_each(|id| {
                                obj.remove_pod(id);
                            });
                        }

                        list_pods
                            .into_iter()
                            .for_each(|report| {
                                let index = obj.len();

                                let mut list = obj.imp().list.borrow_mut();

                                match list.entry(report.id.as_ref().unwrap().to_owned()) {
                                    Entry::Vacant(e) => {
                                        let pod = model::Pod::new(&obj, report);
                                        e.insert(pod.clone());

                                        drop(list);

                                        obj.items_changed(index, 0, 1);
                                        obj.pod_added(&pod);
                                    }
                                    Entry::Occupied(e) => {
                                        let pod = e.get().clone();
                                        drop(list);
                                        pod.update(report);
                                    }
                                }
                            });
                    }
                    Err(e) => {
                        log::error!("Error on retrieving containers: {}", e);
                        err_op(super::RefreshError);
                    }
                }
                obj.set_listing(false);
                obj.set_as_initialized();
            }),
        );
    }

    pub(crate) fn handle_event<F>(&self, event: podman::models::Event, err_op: F)
    where
        F: FnOnce(super::RefreshError) + Clone + 'static,
    {
        let pod_id = event.actor.id;

        match event.action.as_str() {
            "remove" => self.remove_pod(&pod_id),
            _ => self.refresh(self.get_pod(&pod_id).map(|_| pod_id), err_op),
        }
    }

    fn pod_added(&self, pod: &model::Pod) {
        pod.connect_notify_local(
            Some("status"),
            clone!(@weak self as obj => move |_, _| obj.notify("running")),
        );
        self.emit_by_name::<()>("pod-added", &[pod]);
    }

    pub(crate) fn connect_pod_added<F: Fn(&Self, &model::Pod) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("pod-added", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let pod = values[1].get::<model::Pod>().unwrap();
            f(&obj, &pod);

            None
        })
    }
}
