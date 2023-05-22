use adw::subclass::prelude::*;
use adw::traits::BinExt;
use adw::traits::ExpanderRowExt;
use glib::Properties;
use gtk::glib;
use gtk::prelude::*;
use gtk::CompositeTemplate;

use crate::model;
use crate::podman;
use crate::utils;
use crate::view;

const ACTION_PRUNE: &str = "images-prune-page.prune";

mod imp {
    use super::*;

    #[derive(Debug, Default, Properties, CompositeTemplate)]
    #[properties(wrapper_type = super::PrunePage)]
    #[template(resource = "/com/github/marhkb/Pods/ui/images/prune-page.ui")]
    pub(crate) struct PrunePage {
        pub(super) pods_settings: utils::PodsSettings,
        #[property(get, set, construct_only, nullable)]
        pub(super) client: glib::WeakRef<model::Client>,
        #[template_child]
        pub(super) stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub(super) prune_all_switch: TemplateChild<gtk::Switch>,
        #[template_child]
        pub(super) prune_external_switch: TemplateChild<gtk::Switch>,
        #[template_child]
        pub(super) prune_until_expander_row: TemplateChild<view::PruneUntilRow>,
        #[template_child]
        pub(super) action_page_bin: TemplateChild<adw::Bin>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PrunePage {
        const NAME: &'static str = "PdsImagesPrunePage";
        type Type = super::PrunePage;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();

            klass.install_action(ACTION_PRUNE, None, |widget, _, _| {
                widget.prune();
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PrunePage {
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

            self.pods_settings
                .bind("prune-all-images", &*self.prune_all_switch, "active")
                .build();

            self.pods_settings
                .bind(
                    "prune-external-images",
                    &*self.prune_external_switch,
                    "active",
                )
                .build();
        }

        fn dispose(&self) {
            utils::unparent_children(self.obj().upcast_ref());
        }
    }

    impl WidgetImpl for PrunePage {}
}

glib::wrapper! {
    pub(crate) struct PrunePage(ObjectSubclass<imp::PrunePage>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl From<&model::Client> for PrunePage {
    fn from(client: &model::Client) -> Self {
        glib::Object::builder().property("client", client).build()
    }
}

impl PrunePage {
    fn prune(&self) {
        let imp = self.imp();

        let action = self.client().unwrap().action_list().prune_images(
            podman::opts::ImagePruneOpts::builder()
                .all(imp.pods_settings.get("prune-all-images"))
                .external(imp.pods_settings.get("prune-external-images"))
                .filter(if imp.prune_until_expander_row.enables_expansion() {
                    Some(podman::opts::ImagePruneFilter::Until(
                        imp.prune_until_expander_row
                            .prune_until_timestamp()
                            .to_string(),
                    ))
                } else {
                    None
                })
                .build(),
        );

        imp.action_page_bin
            .set_child(Some(&view::ActionPage::from(&action)));
        imp.stack.set_visible_child(&*imp.action_page_bin);
    }
}
