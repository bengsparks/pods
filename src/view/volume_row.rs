use std::cell::RefCell;

use adw::subclass::prelude::ExpanderRowImpl;
use adw::subclass::prelude::PreferencesRowImpl;
use glib::closure;
use glib::Properties;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::CompositeTemplate;

use crate::model;

mod imp {
    use super::*;

    #[derive(Debug, Default, Properties, CompositeTemplate)]
    #[properties(wrapper_type = super::VolumeRow)]
    #[template(file = "volume_row.ui")]
    pub(crate) struct VolumeRow {
        #[property(get, set = Self::set_volume, construct, nullable)]
        pub(super) volume: RefCell<Option<model::Volume>>,
        pub(super) bindings: RefCell<Vec<glib::Binding>>,
        #[template_child]
        pub(super) host_path_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) container_path_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) options_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) writable_switch: TemplateChild<gtk::Switch>,
        #[template_child]
        pub(super) selinux_combo_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub(super) host_path_entry_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub(super) container_path_entry_row: TemplateChild<adw::EntryRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VolumeRow {
        const NAME: &'static str = "PdsVolumeRow";
        type Type = super::VolumeRow;
        type ParentType = adw::ExpanderRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("volume-row.remove", None, |widget, _, _| {
                if let Some(volume) = widget.volume() {
                    volume.remove_request();
                }
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for VolumeRow {
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

            let volume_expr = Self::Type::this_expression("volume");

            volume_expr
                .chain_property::<model::Volume>("host-path")
                .chain_closure::<String>(closure!(|_: Self::Type, path: &str| {
                    let path = path.trim();
                    if path.is_empty() { "?" } else { path }.to_string()
                }))
                .bind(&self.host_path_label.get(), "label", Some(obj));

            volume_expr
                .chain_property::<model::Volume>("container-path")
                .chain_closure::<String>(closure!(|_: Self::Type, path: &str| {
                    let path = path.trim();
                    if path.is_empty() { "?" } else { path }.to_string()
                }))
                .bind(&self.container_path_label.get(), "label", Some(obj));

            gtk::ClosureExpression::new::<String>(
                [
                    volume_expr.chain_property::<model::Volume>("writable"),
                    volume_expr.chain_property::<model::Volume>("selinux"),
                ],
                closure!(
                    |_: Self::Type, writable: bool, selinux: model::VolumeSELinux| {
                        let mut writable = if writable { "rw" } else { "ro" }.to_string();
                        let selinux: &str = selinux.as_ref();
                        if !selinux.is_empty() {
                            writable.push_str(", ");
                            writable.push_str(selinux);
                        }
                        writable
                    }
                ),
            )
            .bind(&self.options_label.get(), "label", Some(obj));
        }
    }

    impl WidgetImpl for VolumeRow {}
    impl ListBoxRowImpl for VolumeRow {}
    impl PreferencesRowImpl for VolumeRow {}
    impl ExpanderRowImpl for VolumeRow {}

    impl VolumeRow {
        pub(super) fn set_volume(&self, value: Option<model::Volume>) {
            let obj = &*self.obj();

            if obj.volume() == value {
                return;
            }

            let mut bindings = self.bindings.borrow_mut();

            while let Some(binding) = bindings.pop() {
                binding.unbind();
            }

            if let Some(ref volume) = value {
                let binding = volume
                    .bind_property("host-path", &*self.host_path_entry_row, "text")
                    .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
                    .build();
                bindings.push(binding);

                let binding = volume
                    .bind_property("container-path", &*self.container_path_entry_row, "text")
                    .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
                    .build();
                bindings.push(binding);

                let binding = volume
                    .bind_property("writable", &*self.writable_switch, "active")
                    .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
                    .build();
                bindings.push(binding);

                let binding = volume
                    .bind_property("selinux", &*self.selinux_combo_row, "selected")
                    .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
                    .transform_to(|_, selinux: model::VolumeSELinux| {
                        Some(
                            match selinux {
                                model::VolumeSELinux::NoLabel => 0_u32,
                                model::VolumeSELinux::Shared => 1_u32,
                                model::VolumeSELinux::Private => 2_u32,
                            }
                            .to_value(),
                        )
                    })
                    .transform_from(|_, position: u32| {
                        Some(
                            match position {
                                0 => model::VolumeSELinux::NoLabel,
                                1 => model::VolumeSELinux::Shared,
                                _ => model::VolumeSELinux::Private,
                            }
                            .to_value(),
                        )
                    })
                    .build();
                bindings.push(binding);
            }

            self.volume.replace(value);
        }
    }
}

glib::wrapper! {
    pub(crate) struct VolumeRow(ObjectSubclass<imp::VolumeRow>)
        @extends gtk::Widget, gtk::ListBoxRow, adw::PreferencesRow, adw::ExpanderRow,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl From<&model::Volume> for VolumeRow {
    fn from(volume: &model::Volume) -> Self {
        glib::Object::builder().property("volume", volume).build()
    }
}
