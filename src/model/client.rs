use futures::StreamExt;
use gtk::glib;
use gtk::glib::clone;
use gtk::prelude::ListModelExtManual;
use gtk::prelude::ParamSpecBuilderExt;
use gtk::prelude::ToValue;
use gtk::subclass::prelude::*;
use once_cell::sync::Lazy;
use once_cell::unsync::OnceCell;

use crate::model;
use crate::model::AbstractContainerListExt;
use crate::monad_boxed_type;
use crate::podman;
use crate::utils;

/// Sync interval in seconds
const SYNC_INTERVAL: u32 = 5;

monad_boxed_type!(pub(crate) BoxedPodman(podman::Podman) impls Debug);

#[derive(Clone, Debug)]
pub(crate) enum ClientError {
    Images,
    Containers,
    Pods,
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct Client {
        pub(super) podman: OnceCell<BoxedPodman>,
        pub(super) connection: OnceCell<model::Connection>,
        pub(super) image_list: OnceCell<model::ImageList>,
        pub(super) container_list: OnceCell<model::ContainerList>,
        pub(super) pod_list: OnceCell<model::PodList>,
        pub(super) action_list: OnceCell<model::ActionList>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Client {
        const NAME: &'static str = "Client";
        type Type = super::Client;
    }

    impl ObjectImpl for Client {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<model::Connection>("connection")
                        .flags(glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY)
                        .build(),
                    glib::ParamSpecBoxed::builder::<BoxedPodman>("podman")
                        .flags(glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY)
                        .build(),
                    glib::ParamSpecObject::builder::<model::ImageList>("image-list")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecObject::builder::<model::ContainerList>("container-list")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecObject::builder::<model::PodList>("pod-list")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecObject::builder::<model::ActionList>("action-list")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                    glib::ParamSpecBoolean::builder("pruning")
                        .flags(glib::ParamFlags::READABLE)
                        .build(),
                ]
            });
            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "connection" => self.connection.set(value.get().unwrap()).unwrap(),
                "podman" => self.podman.set(value.get().unwrap()).unwrap(),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = &*self.obj();
            match pspec.name() {
                "connection" => obj.connection().to_value(),
                "podman" => obj.podman().to_value(),
                "image-list" => obj.image_list().to_value(),
                "container-list" => obj.container_list().to_value(),
                "pod-list" => obj.pod_list().to_value(),
                "action-list" => obj.action_list().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = &*self.obj();

            obj.image_list()
                .connect_image_added(clone!(@weak obj => move |_, image| {
                    obj.container_list()
                        .iter::<model::Container>()
                        .unwrap()
                        .map(|container| container.unwrap())
                        .filter(|container| container.image_id() == Some(image.id()))
                        .for_each(|container| {
                            container.set_image(Some(image));
                            image.add_container(&container);
                        });
                }));

            obj.container_list()
                .connect_container_added(clone!(@weak obj => move |_, container| {
                    let image = obj.image_list().get_image(container.image_id().unwrap());
                    container.set_image(image.as_ref());
                    if let Some(image) = image {
                        image.add_container(container);
                    }

                    if let Some(pod) = container.pod_id().and_then(|id| obj.pod_list().get_pod(id)) {
                        container.set_pod(Some(&pod));
                        pod.container_list().add_container(container);
                    }
                }));
            obj.container_list().connect_container_removed(
                clone!(@weak obj => move |_, container| {
                    if let Some(image) = container
                        .image_id()
                        .and_then(|id| obj.image_list().get_image(id))
                    {
                        image.remove_container(container.id());
                    }

                    if let Some(pod) = container.pod() {
                        pod.container_list().remove_container(container.id());
                    }
                }),
            );

            obj.pod_list()
                .connect_pod_added(clone!(@weak obj => move |_, pod| {
                    obj.container_list()
                        .iter::<model::Container>()
                        .unwrap()
                        .map(|container| container.unwrap())
                        .filter(|container| container.pod_id() == Some(pod.id()))
                        .for_each(|container| {
                            container.set_pod(Some(pod));
                            pod.container_list().add_container(&container);
                        });
                }));
        }
    }
}

glib::wrapper! {
    pub(crate) struct Client(ObjectSubclass<imp::Client>);
}

impl TryFrom<&model::Connection> for Client {
    type Error = podman::Error;

    fn try_from(connection: &model::Connection) -> Result<Self, Self::Error> {
        podman::Podman::new(connection.url()).map(|podman| {
            glib::Object::builder::<Self>()
                .property("connection", connection)
                .property("podman", &BoxedPodman::from(podman))
                .build()
        })
    }
}

impl Client {
    pub(crate) fn podman(&self) -> &BoxedPodman {
        self.imp().podman.get().unwrap()
    }

    pub(crate) fn connection(&self) -> &model::Connection {
        self.imp().connection.get().unwrap()
    }

    pub(crate) fn image_list(&self) -> &model::ImageList {
        self.imp()
            .image_list
            .get_or_init(|| model::ImageList::from(Some(self)))
    }

    pub(crate) fn container_list(&self) -> &model::ContainerList {
        self.imp()
            .container_list
            .get_or_init(|| model::ContainerList::from(Some(self)))
    }

    pub(crate) fn pod_list(&self) -> &model::PodList {
        self.imp()
            .pod_list
            .get_or_init(|| model::PodList::from(Some(self)))
    }

    pub(crate) fn action_list(&self) -> &model::ActionList {
        self.imp()
            .action_list
            .get_or_init(|| model::ActionList::from(Some(self)))
    }

    pub(crate) fn check_service<T, E, F>(&self, op: T, err_op: E, finish_op: F)
    where
        T: FnOnce() + 'static,
        E: FnOnce(ClientError) + Clone + 'static,
        F: FnOnce(podman::Error) + Clone + 'static,
    {
        utils::do_async(
            {
                let podman = self.podman().clone();
                async move { podman.ping().await }
            },
            clone!(@weak self as obj => move |result| match result {
                Ok(_) => {
                    obj.image_list().refresh({
                        let err_op = err_op.clone();
                        |_| err_op(ClientError::Images)
                    });
                    obj.container_list().refresh(
                        None,
                        {
                            let err_op = err_op.clone();
                            |_| err_op(ClientError::Containers)
                        }
                    );
                    obj.pod_list().refresh(
                        None,
                        {
                            let err_op = err_op.clone();
                            |_| err_op(ClientError::Pods)
                        }
                    );

                    op();
                    obj.start_event_listener(err_op, finish_op);
                    obj.start_refresh_interval();
                }
                Err(e) => {
                    log::error!("Could not connect to Podman: {e}");
                    // No need to show a toast. The start service page is enough.
                }
            }),
        );
    }

    fn start_event_listener<E, F>(&self, err_op: E, finish_op: F)
    where
        E: FnOnce(ClientError) + Clone + 'static,
        F: FnOnce(podman::Error) + Clone + 'static,
    {
        utils::run_stream(
            self.podman().clone(),
            |podman| {
                podman
                    .events(&podman::opts::EventsOpts::builder().build())
                    .boxed()
            },
            clone!(
                @weak self as obj => @default-return glib::Continue(false),
                move |result: podman::Result<podman::models::Event>|
            {
                glib::Continue(match result {
                    Ok(event) => {
                        log::debug!("Event: {event:?}");
                        match event.typ.as_str() {
                            "image" => obj.image_list().handle_event(event, {
                                let err_op = err_op.clone();
                                |_| err_op(ClientError::Images)
                            }),
                            "container" => obj.container_list().handle_event(event, {
                                let err_op = err_op.clone();
                                |_| err_op(ClientError::Containers)
                            }),
                            "pod" => obj.pod_list().handle_event(event, {
                                let err_op = err_op.clone();
                                |_| err_op(ClientError::Pods)
                            }),
                            other => log::warn!("Unhandled event type: {other}"),
                        }
                        true
                    }
                    Err(e) => {
                        log::error!("Stopping image event stream due to error: {e}");
                        finish_op.clone()(e);
                        false
                    }
                })
            }),
        );
    }

    /// This is needed to keep track of images and containers that are managed by Buildah.
    /// See https://github.com/marhkb/pods/issues/306
    fn start_refresh_interval(&self) {
        glib::timeout_add_seconds_local(
            SYNC_INTERVAL,
            clone!(@weak self as obj => @default-return glib::Continue(false), move || {
                log::debug!("Syncing images, containers and pods");

                obj.image_list().refresh(|_| {});
                obj.container_list().refresh(None, |_| {});
                obj.pod_list().refresh(None, |_| {});

                log::debug!("Sleeping for {SYNC_INTERVAL} until next sync");

                glib::Continue(true)
            }),
        );
    }
}
