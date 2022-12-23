use std::cell::RefCell;
use std::io::Read;
use std::path::PathBuf;

use gettextrs::gettext;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::glib::clone;
use gtk::prelude::Cast;
use gtk::prelude::ListModelExt;
use gtk::prelude::ObjectExt;
use gtk::prelude::ParamSpecBuilderExt;
use gtk::prelude::SettingsExt;
use gtk::prelude::StaticType;
use gtk::prelude::ToValue;
use gtk::subclass::prelude::*;
use indexmap::IndexMap;
use once_cell::sync::Lazy;
use tokio::io::AsyncWriteExt;

use crate::model;
use crate::podman;
use crate::utils;
use crate::utils::config_dir;
use crate::RUNTIME;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct ConnectionManager {
        pub(super) settings: utils::PodsSettings,
        pub(super) connections: RefCell<IndexMap<String, model::Connection>>,
        pub(super) client: RefCell<Option<model::Client>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ConnectionManager {
        const NAME: &'static str = "ConnectionManager";
        type Type = super::ConnectionManager;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for ConnectionManager {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<model::Client>("client")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("connecting")
                        .read_only()
                        .build(),
                ]
            });
            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = &*self.obj();
            match pspec.name() {
                "client" => obj.client().to_value(),
                "connecting" => obj.is_connecting().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for ConnectionManager {
        fn item_type(&self) -> glib::Type {
            model::Connection::static_type()
        }

        fn n_items(&self) -> u32 {
            self.connections.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.connections
                .borrow()
                .get_index(position as usize)
                .map(|(_, obj)| obj.upcast_ref())
                .cloned()
        }
    }
}

glib::wrapper! {
    pub(crate) struct ConnectionManager(ObjectSubclass<imp::ConnectionManager>)
        @implements gio::ListModel;
}

impl Default for ConnectionManager {
    fn default() -> Self {
        glib::Object::builder().build()
    }
}

impl ConnectionManager {
    pub(crate) fn setup(&self) -> anyhow::Result<()> {
        let connections = self.load_from_disk()?;
        let connections_len = connections.len();

        let imp = self.imp();

        imp.connections.borrow_mut().extend(
            connections
                .into_iter()
                .map(|(uuid, conn)| (uuid, model::Connection::from_connection_info(&conn, self))),
        );

        self.items_changed(
            (imp.connections.borrow().len() - connections_len) as u32,
            0,
            connections_len as u32,
        );

        if self.n_items() > 0 {
            let last_used_connection = imp.settings.string("last-used-connection");
            self.set_client_from(last_used_connection.as_str())?;
        }

        Ok(())
    }

    fn load_from_disk(&self) -> anyhow::Result<IndexMap<String, model::ConnectionInfo>> {
        if utils::config_dir().exists() {
            let path = path();

            if path.exists() {
                let mut file = std::fs::OpenOptions::new().read(true).open(path)?;

                let mut buf = vec![];
                file.read_to_end(&mut buf)?;

                serde_json::from_slice::<IndexMap<String, model::ConnectionInfo>>(&buf)
                    .map_err(anyhow::Error::from)
            } else {
                Ok(IndexMap::default())
            }
        } else {
            std::fs::create_dir_all(config_dir())?;
            Ok(IndexMap::default())
        }
    }

    pub(crate) fn sync_to_disk<F>(&self, op: F)
    where
        F: FnOnce(anyhow::Result<()>) + 'static,
    {
        let value = self
            .imp()
            .connections
            .borrow()
            .iter()
            .map(|(key, connection)| (key.to_owned(), model::ConnectionInfo::from(connection)))
            .collect::<IndexMap<_, _>>();

        let buf = serde_json::to_vec_pretty(&value).unwrap();

        utils::do_async(
            async move {
                if !utils::config_dir().exists() {
                    tokio::fs::create_dir_all(&config_dir()).await?;
                }

                let mut file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(path())
                    .await?;

                file.write_all(&buf).await.map_err(anyhow::Error::from)
            },
            clone!(@weak self as obj => move |result| op(result)),
        );
    }

    pub(crate) fn try_connect<F>(
        &self,
        name: &str,
        url: &str,
        rgb: Option<gdk::RGBA>,
        op: F,
    ) -> anyhow::Result<()>
    where
        F: FnOnce(podman::Result<podman::models::LibpodPingInfo>) + 'static,
    {
        let imp = self.imp();

        if imp.connections.borrow().values().any(|c| c.name() == name) {
            return Err(anyhow::anyhow!(gettext!(
                "Connection '{}' already exists.",
                name
            )));
        }

        let connection =
            model::Connection::new(glib::uuid_string_random().as_str(), name, url, rgb, self);

        let client = model::Client::try_from(&connection)?;

        utils::do_async(
            {
                let podman = client.podman().clone();
                async move { podman.ping().await }
            },
            clone!(@weak self as obj => move |result| {
                match &result {
                    Ok(_) => {
                        obj.set_client(Some(client));

                        let (position, _) = obj.imp()
                            .connections
                            .borrow_mut()
                            .insert_full(connection.uuid().to_owned(), connection.clone());

                        obj.items_changed(position as u32, 0, 1);

                        obj.sync_to_disk(|_| {});
                    }
                    Err(e) => log::error!("Error on pinging connection: {e}"),
                }
                op(result);
            }),
        );

        Ok(())
    }

    pub(crate) fn remove_connection(&self, uuid: &str) {
        let mut connections = self.imp().connections.borrow_mut();
        if let Some((position, _, _)) = connections.shift_remove_full(uuid) {
            drop(connections);

            if self
                .client()
                .map(|client| client.connection().uuid() == uuid)
                .unwrap_or(false)
            {
                self.set_client(None);
            }

            self.items_changed(position as u32, 1, 0);
            self.sync_to_disk(|_| {});
        }
    }

    pub(crate) fn contains_local_connection(&self) -> bool {
        self.imp()
            .connections
            .borrow()
            .values()
            .any(model::Connection::is_local)
    }

    pub(crate) fn client(&self) -> Option<model::Client> {
        self.imp().client.borrow().clone()
    }

    pub(crate) fn set_client_from(&self, connection_uuid: &str) -> anyhow::Result<()> {
        if self
            .client()
            .map(|c| c.connection().uuid() == connection_uuid)
            .unwrap_or(false)
        {
            return Ok(());
        }

        let connection = self
            .connection_by_uuid(connection_uuid)
            .ok_or_else(|| anyhow::anyhow!("connection not found"))?;

        let client = model::Client::try_from(&connection)?;

        RUNTIME.block_on(client.podman().ping())?;

        self.set_client(Some(client));

        Ok(())
    }

    fn set_client(&self, value: Option<model::Client>) {
        let imp = self.imp();

        if let Some(ref client) = value {
            if let Err(e) = imp
                .settings
                .set_string("last-used-connection", client.connection().uuid())
            {
                log::error!("Could not write last used connection {e}");
            }
        }

        imp.client.replace(value);
        self.notify("client");
    }

    pub(crate) fn unset_client(&self) {
        self.set_client(None);
    }

    pub(crate) fn is_connecting(&self) -> bool {
        self.imp()
            .connections
            .borrow()
            .values()
            .any(model::Connection::is_connecting)
    }

    pub(crate) fn connection_by_uuid(&self, uuid: &str) -> Option<model::Connection> {
        self.imp().connections.borrow_mut().get(uuid).cloned()
    }
}

fn path() -> PathBuf {
    utils::config_dir().join("connections.json")
}
