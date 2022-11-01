use std::cell::Cell;
use std::cell::RefCell;

use anyhow::anyhow;
use futures::StreamExt;
use gtk::gio;
use gtk::glib;
use gtk::glib::clone;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use indexmap::map::Entry;
use indexmap::map::IndexMap;
use once_cell::sync::Lazy;
use once_cell::unsync::OnceCell;

use crate::model;
use crate::model::AbstractContainerListExt;
use crate::model::SelectableListExt;
use crate::podman;
use crate::utils;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct ContainerList {
        pub(super) client: glib::WeakRef<model::Client>,
        pub(super) list: RefCell<IndexMap<String, model::Container>>,
        pub(super) listing: Cell<bool>,
        pub(super) initialized: OnceCell<()>,
        pub(super) selection_mode: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContainerList {
        const NAME: &'static str = "ContainerList";
        type Type = super::ContainerList;
        type Interfaces = (
            gio::ListModel,
            model::AbstractContainerList,
            model::SelectableList,
        );
    }

    impl ObjectImpl for ContainerList {
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
                    glib::ParamSpecUInt::builder("created")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecUInt::builder("dead")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecUInt::builder("exited")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecUInt::builder("paused")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecUInt::builder("removing")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecUInt::builder("running")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecUInt::builder("stopped")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecUInt::builder("stopping")
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
            let obj = &*self.obj();
            match pspec.name() {
                "client" => obj.client().to_value(),
                "len" => obj.len().to_value(),
                "listing" => obj.listing().to_value(),
                "initialized" => obj.is_initialized().to_value(),
                "created" => obj.created().to_value(),
                "dead" => obj.dead().to_value(),
                "exited" => obj.exited().to_value(),
                "paused" => obj.paused().to_value(),
                "removing" => obj.removing().to_value(),
                "running" => obj.running().to_value(),
                "stopped" => obj.stopped().to_value(),
                "stopping" => obj.stopping().to_value(),
                "selection-mode" => self.selection_mode.get().to_value(),
                "num-selected" => obj.num_selected().to_value(),
                _ => unimplemented!(),
            }
        }
        fn constructed(&self) {
            self.parent_constructed();

            let obj = &*self.obj();

            model::AbstractContainerList::bootstrap(obj);
            model::SelectableList::bootstrap(obj);

            utils::run_stream(
                obj.client().unwrap().podman().containers(),
                |containers| {
                    containers
                        .stats_stream(
                            &podman::opts::ContainerStatsOptsBuilder::default()
                                .interval(1)
                                .build(),
                        )
                        .boxed()
                },
                clone!(
                    @weak obj => @default-return glib::Continue(false),
                    move |result: podman::Result<podman::models::ContainerStats200Response>|
                {
                    match result
                        .map_err(anyhow::Error::from)
                        .and_then(|mut value| {
                            value
                                .as_object_mut()
                                .and_then(|object| object.remove("Stats"))
                                .ok_or_else(|| anyhow!("Field 'Stats' is not present"))
                        })
                        .and_then(|value| {
                            serde_json::from_value::<Vec<podman::models::ContainerStats>>(value)
                                .map_err(anyhow::Error::from)
                        }) {
                        Ok(stats) => {
                            stats.into_iter().for_each(|stat| {
                                if let Some(container) =
                                    obj.get_container(stat.container_id.as_ref().unwrap())
                                {
                                    if container.status() == model::ContainerStatus::Running {
                                        container.set_stats(
                                            Some(model::BoxedContainerStats::from(stat))
                                        );
                                    }
                                }
                            });
                        }
                        Err(e) => log::warn!("Error occurred on receiving stats stream element: {e}"),
                    }

                    glib::Continue(true)
                }),
            );

            obj.client().unwrap().podman();
        }
    }

    impl ListModelImpl for ContainerList {
        fn item_type(&self) -> glib::Type {
            model::Container::static_type()
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
    pub(crate) struct ContainerList(ObjectSubclass<imp::ContainerList>)
        @implements gio::ListModel, model::AbstractContainerList, model::SelectableList;
}

impl From<Option<&model::Client>> for ContainerList {
    fn from(client: Option<&model::Client>) -> Self {
        glib::Object::builder::<Self>()
            .property("client", &client)
            .build()
    }
}

impl ContainerList {
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

    pub(crate) fn created(&self) -> u32 {
        self.num_containers_of_status(model::ContainerStatus::Created)
    }

    pub(crate) fn dead(&self) -> u32 {
        self.num_containers_of_status(model::ContainerStatus::Dead)
    }

    pub(crate) fn exited(&self) -> u32 {
        self.num_containers_of_status(model::ContainerStatus::Exited)
    }

    pub(crate) fn paused(&self) -> u32 {
        self.num_containers_of_status(model::ContainerStatus::Paused)
    }

    pub(crate) fn removing(&self) -> u32 {
        self.num_containers_of_status(model::ContainerStatus::Removing)
    }

    pub(crate) fn running(&self) -> u32 {
        self.num_containers_of_status(model::ContainerStatus::Running)
    }

    pub(crate) fn stopped(&self) -> u32 {
        self.num_containers_of_status(model::ContainerStatus::Stopped)
    }

    pub(crate) fn stopping(&self) -> u32 {
        self.num_containers_of_status(model::ContainerStatus::Stopping)
    }

    pub(crate) fn num_containers_of_status(&self, status: model::ContainerStatus) -> u32 {
        self.imp()
            .list
            .borrow()
            .values()
            .filter(|container| container.status() == status)
            .count() as u32
    }

    pub(crate) fn get_container(&self, id: &str) -> Option<model::Container> {
        self.imp().list.borrow().get(id).cloned()
    }

    pub(crate) fn remove_container(&self, id: &str) {
        let mut list = self.imp().list.borrow_mut();
        if let Some((idx, _, container)) = list.shift_remove_full(id) {
            container.on_deleted();
            drop(list);
            self.container_removed(&container);
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
                        .containers()
                        .list(
                            &podman::opts::ContainerListOpts::builder()
                                .all(true)
                                .filter(
                                    id.map(podman::Id::from)
                                        .map(podman::opts::ContainerListFilter::Id),
                                )
                                .build(),
                        )
                        .await
                }
            },
            clone!(@weak self as obj => move |result| {
                match result {
                    Ok(list_containers) => {
                        if id.is_none() {
                            let to_remove = obj
                                .imp()
                                .list
                                .borrow()
                                .keys()
                                .filter(|id| {
                                    !list_containers
                                        .iter()
                                        .any(|list_container| list_container.id.as_ref() == Some(id))
                                })
                                .cloned()
                                .collect::<Vec<_>>();
                            to_remove.iter().for_each(|id| {
                                obj.remove_container(id);
                            });
                        }

                        list_containers
                            .into_iter()
                            .filter(|list_container| !list_container.is_infra.unwrap_or_default())
                            .for_each(|list_container| {
                                let index = obj.len();

                                let mut list = obj.imp().list.borrow_mut();

                                match list.entry(list_container.id.as_ref().unwrap().to_owned()) {
                                    Entry::Vacant(e) => {
                                        let container = model::Container::new(&obj, list_container);
                                        e.insert(container.clone());

                                        drop(list);

                                        obj.items_changed(index, 0, 1);
                                        obj.container_added(&container);
                                    }
                                    Entry::Occupied(e) => {
                                        let container = e.get().clone();
                                        drop(list);
                                        container.update(list_container);
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
        let container_id = event.actor.id;

        match event.action.as_str() {
            "remove" => self.remove_container(&container_id),
            "health_status" => {
                if let Some(container) = self.get_container(&container_id) {
                    container.inspect(|_| {});
                }
            }
            _ => self.refresh(
                self.get_container(&container_id).map(|_| container_id),
                err_op,
            ),
        }
    }
}
