use std::cell::RefCell;

use adw::subclass::prelude::PreferencesGroupImpl;
use adw::traits::BinExt;
use gettextrs::gettext;
use gettextrs::ngettext;
use glib::clone;
use glib::closure;
use glib::Properties;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::CompositeTemplate;
use once_cell::sync::OnceCell as SyncOnceCell;
use once_cell::unsync::OnceCell as UnsyncOnceCell;

use crate::model;
use crate::model::AbstractContainerListExt;
use crate::utils;
use crate::view;

mod imp {
    use super::*;

    #[derive(Debug, Default, Properties, CompositeTemplate)]
    #[properties(wrapper_type = super::ContainersGroup)]
    #[template(file = "containers_group.ui")]
    pub(crate) struct ContainersGroup {
        pub(super) settings: utils::PodsSettings,
        pub(super) properties_filter: UnsyncOnceCell<gtk::Filter>,
        pub(super) sorter: UnsyncOnceCell<gtk::Sorter>,
        #[property(get, set, nullable)]
        pub(super) no_containers_label: RefCell<Option<String>>,
        #[property(get, set = Self::set_show_running_settings_key)]
        pub(super) show_running_settings_key: RefCell<String>,
        #[property(get, set = Self::set_container_list, nullable)]
        pub(super) container_list: glib::WeakRef<model::AbstractContainerList>,
        #[template_child]
        pub(super) create_container_row: TemplateChild<gtk::ListBoxRow>,
        #[template_child]
        pub(super) header_suffix_prefix_bin: TemplateChild<adw::Bin>,
        #[template_child]
        pub(super) header_suffix_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) show_only_running_switch: TemplateChild<gtk::Switch>,
        #[template_child]
        pub(super) create_container_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) list_box: TemplateChild<gtk::ListBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContainersGroup {
        const NAME: &'static str = "PdsContainersGroup";
        type Type = super::ContainersGroup;
        type ParentType = adw::PreferencesGroup;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ContainersGroup {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: SyncOnceCell<Vec<glib::ParamSpec>> = SyncOnceCell::new();
            PROPERTIES.get_or_init(|| {
                Self::derived_properties()
                    .iter()
                    .cloned()
                    .chain(vec![glib::ParamSpecObject::builder::<gtk::Widget>(
                        "header-suffix-prefix",
                    )
                    .explicit_notify()
                    .build()])
                    .collect::<Vec<_>>()
            })
        }

        fn set_property(&self, id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "header-suffix-prefix" => self.obj().set_header_suffix_prefix(value.get().unwrap()),
                _ => self.derived_set_property(id, value, pspec),
            }
        }

        fn property(&self, id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "header-suffix-prefix" => self.obj().header_suffix_prefix().to_value(),
                _ => self.derived_property(id, pspec),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = &*self.obj();

            let container_list_expr = Self::Type::this_expression("container-list");
            let container_list_len_expr =
                container_list_expr.chain_property::<model::AbstractContainerList>("len");
            let is_selection_mode_expr = container_list_expr
                .chain_property::<model::ContainerList>("selection-mode")
                .chain_closure::<bool>(closure!(|_: Self::Type, selection_mode: bool| {
                    !selection_mode
                }));

            container_list_len_expr
                .chain_closure::<bool>(closure!(|_: Self::Type, len: u32| len > 0))
                .bind(&*self.header_suffix_box, "visible", Some(obj));

            is_selection_mode_expr.bind(&*self.create_container_button, "visible", Some(obj));
            is_selection_mode_expr.bind(&*self.create_container_row, "visible", Some(obj));

            gtk::ClosureExpression::new::<Option<String>>(
                &[
                    container_list_len_expr,
                    container_list_expr.chain_property::<model::AbstractContainerList>("running"),
                ],
                closure!(|obj: Self::Type, len: u32, running: u32| {
                    if len == 0 {
                        obj.no_containers_label()
                    } else {
                        Some(if len == 1 {
                            if running == 1 {
                                gettext("1 container, running")
                            } else {
                                gettext("1 container, stopped")
                            }
                        } else {
                            ngettext!(
                                // Translators: There's a wide space (U+2002) between ", {}".
                                "{} container total, {} running",
                                "{} containers total, {} running",
                                len,
                                len,
                                running,
                            )
                        })
                    }
                }),
            )
            .bind(obj, "description", Some(obj));

            let properties_filter = gtk::AnyFilter::new();
            properties_filter.append(gtk::CustomFilter::new(
                clone!(@weak obj => @default-return false, move |_| {
                    !obj.imp().show_only_running_switch.is_active()
                }),
            ));
            properties_filter.append(gtk::BoolFilter::new(Some(
                model::Container::this_expression("status").chain_closure::<bool>(closure!(
                    |_: model::Container, status: model::ContainerStatus| status
                        == model::ContainerStatus::Running
                )),
            )));

            let sorter = gtk::CustomSorter::new(|item1, item2| {
                item1
                    .downcast_ref::<model::Container>()
                    .unwrap()
                    .name()
                    .to_lowercase()
                    .cmp(
                        &item2
                            .downcast_ref::<model::Container>()
                            .unwrap()
                            .name()
                            .to_lowercase(),
                    )
                    .into()
            });

            self.properties_filter
                .set(properties_filter.upcast())
                .unwrap();
            self.sorter.set(sorter.upcast()).unwrap();

            self.show_only_running_switch.connect_active_notify(
                clone!(@weak obj => move |switch| {
                    obj.update_properties_filter(
                        if switch.is_active() {
                            gtk::FilterChange::MoreStrict
                        } else {
                            gtk::FilterChange::LessStrict
                        }
                    );
                }),
            );
        }
    }

    impl WidgetImpl for ContainersGroup {}
    impl PreferencesGroupImpl for ContainersGroup {}

    impl ContainersGroup {
        pub(super) fn set_show_running_settings_key(&self, value: String) {
            let obj = &*self.obj();
            if obj.show_running_settings_key() == value {
                return;
            }

            self.settings
                .bind(&value, &*self.show_only_running_switch, "active")
                .build();

            self.show_running_settings_key.replace(value);
        }

        pub(super) fn set_container_list(&self, value: Option<&model::AbstractContainerList>) {
            let obj = &*self.obj();
            if obj.container_list().as_ref() == value {
                return;
            }

            if let Some(value) = value {
                value.connect_notify_local(
                    Some("running"),
                    clone!(@weak obj => move |_, _| {
                        obj.update_properties_filter(gtk::FilterChange::Different)
                    }),
                );

                value.connect_container_name_changed(clone!(@weak obj => move |_, _| {
                    glib::timeout_add_seconds_local_once(
                        1,
                        clone!(@weak obj => move || obj.update_sorter()),
                    );
                }));

                let model = gtk::SortListModel::new(
                    Some(gtk::FilterListModel::new(
                        Some(value.to_owned()),
                        self.properties_filter.get().cloned(),
                    )),
                    self.sorter.get().cloned(),
                );

                self.list_box.bind_model(Some(&model), |item| {
                    view::ContainerRow::from(item.downcast_ref().unwrap()).upcast()
                });
                self.list_box.append(&*self.create_container_row);
            }

            self.container_list.set(value);
        }
    }
}

glib::wrapper! {
    pub(crate) struct ContainersGroup(ObjectSubclass<imp::ContainersGroup>)
        @extends gtk::Widget, adw::PreferencesGroup,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ContainersGroup {
    fn default() -> Self {
        glib::Object::builder().build()
    }
}

impl ContainersGroup {
    pub(crate) fn action_create_container() -> &'static str {
        "containers-group.create-container"
    }

    pub(crate) fn header_suffix_prefix(&self) -> Option<gtk::Widget> {
        self.imp().header_suffix_prefix_bin.child()
    }

    pub(crate) fn set_header_suffix_prefix(&self, value: Option<&gtk::Widget>) {
        if self.header_suffix_prefix().as_ref() == value {
            return;
        }
        self.imp().header_suffix_prefix_bin.set_child(value);
        self.notify("header-suffix-prefix");
    }

    fn update_properties_filter(&self, filter_change: gtk::FilterChange) {
        self.imp()
            .properties_filter
            .get()
            .unwrap()
            .changed(filter_change);
    }

    fn update_sorter(&self) {
        self.imp()
            .sorter
            .get()
            .unwrap()
            .changed(gtk::SorterChange::Different);
    }
}
