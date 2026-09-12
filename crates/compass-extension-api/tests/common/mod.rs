//! Fixtures shared by the integration tests.

#![allow(dead_code)]

use compass_extension_api::*;
use std::collections::BTreeMap;

pub fn image() -> Image {
    Image {
        source: ImageSource::Themed {
            light: Box::new(ImageSource::Asset("light.png".into())),
            dark: Box::new(ImageSource::Builtin("moon".into())),
        },
        fallback: Some(ImageSource::FileIcon("/etc/hosts".into())),
        tint: Some(Color::Literal("#ff8800".into())),
        mask: Some(ImageMask::Circle),
    }
}

pub fn panel() -> ActionPanel {
    ActionPanel {
        id: NodeId::ROOT,
        title: Some("Actions".into()),
        sections: vec![
            ActionSection {
                id: NodeId::ROOT,
                title: None,
                items: vec![
                    ActionItem::Action(
                        Action::new("Open", "h.open")
                            .with_key("open")
                            .with_shortcut(Shortcut::new([KeyModifier::Cmd], "return")),
                    ),
                    ActionItem::Action(Action::new("Copy Link", "h.copy")),
                ],
            },
            ActionSection {
                id: NodeId::ROOT,
                title: Some("Danger".into()),
                items: vec![ActionItem::Submenu(ActionSubmenu {
                    id: NodeId::ROOT,
                    key: Some("danger".into()),
                    title: "Remove…".into(),
                    icon: Some(Image::builtin("trash")),
                    shortcut: Some(Shortcut::new([KeyModifier::Cmd], "backspace")),
                    on_open: Some(HandlerId::new("h.danger.open")),
                    items: vec![ActionItem::Action(Action {
                        style: ActionStyle::Destructive,
                        shortcut: Some(Shortcut::new([KeyModifier::Ctrl, KeyModifier::Shift], "x")),
                        ..Action::new("Delete Forever", "h.delete")
                    })],
                })],
            },
        ],
    }
}

pub fn detail() -> Detail {
    Detail {
        id: NodeId::ROOT,
        markdown: Some("# Title\n\nbody".into()),
        metadata: vec![
            MetadataItem::Label {
                title: "Status".into(),
                text: "Open".into(),
                icon: Some(Image::builtin("circle")),
                color: Some(Color::Named("green".into())),
            },
            MetadataItem::Separator,
            MetadataItem::Link {
                title: "Home".into(),
                text: "example".into(),
                target: "https://example.invalid".into(),
            },
            MetadataItem::TagList {
                title: "Labels".into(),
                tags: vec![MetadataTag {
                    text: "bug".into(),
                    icon: None,
                    color: Some(Color::Literal("#c00".into())),
                    handler: Some(HandlerId::new("h.tag.bug")),
                }],
            },
        ],
        is_loading: false,
        navigation_title: Some("Issue".into()),
        actions: Some(panel()),
    }
}

pub fn empty_state() -> EmptyState {
    EmptyState {
        id: NodeId::ROOT,
        title: "Nothing here".into(),
        description: Some("Try another query".into()),
        icon: Some(Image::builtin("magnifier")),
        actions: Some(ActionPanel::of([Action::new("Reload", "h.reload")])),
    }
}

pub fn search_bar() -> SearchBar {
    SearchBar {
        placeholder: Some("Search issues".into()),
        text: Some("bug".into()),
        echo: None,
        host_filtering: true,
        on_change: Some(HandlerId::new("h.search")),
        accessory: Some(dropdown()),
    }
}

pub fn dropdown() -> Dropdown {
    Dropdown {
        placeholder: Some("All".into()),
        value: Some("open".into()),
        echo: None,
        sections: vec![DropdownSection {
            title: Some("State".into()),
            options: vec![
                DropdownOption {
                    title: "Open".into(),
                    value: "open".into(),
                    icon: None,
                    keywords: vec!["o".into()],
                },
                DropdownOption {
                    title: "Closed".into(),
                    value: "closed".into(),
                    icon: Some(Image::builtin("check")),
                    keywords: vec![],
                },
            ],
        }],
        filtering: true,
        on_change: Some(HandlerId::new("h.filter")),
    }
}

pub fn list_view() -> ListView {
    ListView {
        id: NodeId::ROOT,
        navigation_title: Some("Issues".into()),
        is_loading: false,
        show_detail: true,
        selected: None,
        on_selection_change: Some(HandlerId::new("h.select")),
        search: search_bar(),
        sections: vec![
            ListSection {
                id: NodeId::ROOT,
                key: Some("recent".into()),
                title: Some("Recent".into()),
                subtitle: Some("last 7 days".into()),
                items: vec![
                    ListItem {
                        accessories: vec![Accessory {
                            text: Some("2d".into()),
                            tag: Some("P1".into()),
                            color: Some(Color::Named("red".into())),
                            icon: Some(image()),
                            tooltip: Some("priority".into()),
                        }],
                        keywords: vec!["crash".into()],
                        detail: Some(detail()),
                        subtitle: Some("COMP-1".into()),
                        icon: Some(image()),
                        ..ListItem::new("Crash on launch")
                            .with_key("COMP-1")
                            .with_actions(panel())
                    },
                    ListItem::new("Slow search").with_key("COMP-2"),
                ],
            },
            ListSection::untitled([ListItem::new("Unkeyed row")]),
        ],
        actions: Some(ActionPanel::of([Action::new("Refresh", "h.refresh")])),
        empty_state: Some(empty_state()),
        pagination: Some(Pagination {
            has_more: true,
            loaded: 20,
            on_load_more: HandlerId::new("h.more"),
        }),
    }
}

pub fn grid_view() -> GridView {
    GridView {
        id: NodeId::ROOT,
        navigation_title: Some("Colours".into()),
        is_loading: true,
        columns: Some(5),
        aspect_ratio: Some(AspectRatio {
            width: 16,
            height: 9,
        }),
        inset: GridInset::Medium,
        fit: GridFit::Fill,
        selected: None,
        on_selection_change: None,
        search: SearchBar::default(),
        sections: vec![GridSection {
            id: NodeId::ROOT,
            key: None,
            title: Some("Warm".into()),
            subtitle: None,
            columns: Some(3),
            aspect_ratio: Some(AspectRatio {
                width: 1,
                height: 1,
            }),
            inset: Some(GridInset::Large),
            fit: Some(GridFit::Contain),
            items: vec![
                GridItem {
                    id: NodeId::ROOT,
                    key: Some("red".into()),
                    title: "Red".into(),
                    subtitle: Some("#f00".into()),
                    content: GridContent::Color(Color::Literal("#ff0000".into())),
                    tooltip: Some("warm".into()),
                    keywords: vec!["hot".into()],
                    actions: Some(ActionPanel::of([Action::new("Pick", "h.pick.red")])),
                },
                GridItem {
                    id: NodeId::ROOT,
                    key: None,
                    title: "Photo".into(),
                    subtitle: None,
                    content: GridContent::Image(image()),
                    tooltip: None,
                    keywords: vec![],
                    actions: None,
                },
            ],
        }],
        actions: Some(panel()),
        empty_state: Some(empty_state()),
        pagination: None,
    }
}

pub fn form_view() -> FormView {
    FormView {
        id: NodeId::ROOT,
        navigation_title: Some("New Issue".into()),
        is_loading: false,
        enable_drafts: true,
        items: vec![
            FormItem::Description {
                id: NodeId::ROOT,
                title: Some("About".into()),
                text: "Fill this in".into(),
            },
            FormItem::Separator { id: NodeId::ROOT },
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "title".into(),
                title: Some("Title".into()),
                error: None,
                info: Some("Short summary".into()),
                autofocus: true,
                value: Some(FieldValue::Text("Crash".into())),
                echo: None,
                on_change: Some(HandlerId::new("h.title")),
                kind: FieldKind::Text {
                    placeholder: Some("Summary".into()),
                },
            })),
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "secret".into(),
                title: None,
                error: Some("required".into()),
                info: None,
                autofocus: false,
                value: None,
                echo: None,
                on_change: None,
                kind: FieldKind::Password { placeholder: None },
            })),
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "body".into(),
                title: None,
                error: None,
                info: None,
                autofocus: false,
                value: Some(FieldValue::Empty),
                echo: None,
                on_change: None,
                kind: FieldKind::TextArea {
                    placeholder: None,
                    markdown: true,
                },
            })),
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "urgent".into(),
                title: None,
                error: None,
                info: None,
                autofocus: false,
                value: Some(FieldValue::Bool(true)),
                echo: None,
                on_change: None,
                kind: FieldKind::Checkbox {
                    label: Some("Urgent".into()),
                },
            })),
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "state".into(),
                title: None,
                error: None,
                info: None,
                autofocus: false,
                value: Some(FieldValue::Values(vec!["open".into()])),
                echo: None,
                on_change: None,
                kind: FieldKind::Dropdown(dropdown()),
            })),
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "labels".into(),
                title: None,
                error: None,
                info: None,
                autofocus: false,
                value: None,
                echo: None,
                on_change: None,
                kind: FieldKind::TagPicker {
                    options: vec![DropdownOption {
                        title: "bug".into(),
                        value: "bug".into(),
                        icon: None,
                        keywords: vec![],
                    }],
                    placeholder: None,
                },
            })),
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "due".into(),
                title: None,
                error: None,
                info: None,
                autofocus: false,
                value: Some(FieldValue::Date("2026-09-12T00:00:00Z".into())),
                echo: None,
                on_change: None,
                kind: FieldKind::DatePicker {
                    min: None,
                    max: None,
                    precision: DatePrecision::Minute,
                },
            })),
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "attachments".into(),
                title: None,
                error: None,
                info: None,
                autofocus: false,
                value: Some(FieldValue::Paths(vec!["/tmp/a".into()])),
                echo: None,
                on_change: None,
                kind: FieldKind::FilePicker {
                    allow_multiple: true,
                    allow_directories: false,
                    allow_files: true,
                },
            })),
            FormItem::Field(Box::new(FormField {
                id: NodeId::ROOT,
                name: "count".into(),
                title: None,
                error: None,
                info: None,
                autofocus: false,
                value: Some(FieldValue::Integer(-3)),
                echo: None,
                on_change: None,
                kind: FieldKind::Text { placeholder: None },
            })),
        ],
        actions: Some(ActionPanel::of([
            Action::new("Submit", "h.submit").with_shortcut(Shortcut::bare("return"))
        ])),
    }
}

pub fn every_view() -> Vec<View> {
    vec![
        View::List(list_view()),
        View::Grid(grid_view()),
        View::Detail(detail()),
        View::Form(form_view()),
    ]
}

pub fn form_values() -> BTreeMap<String, FieldValue> {
    let mut m = BTreeMap::new();
    m.insert("title".into(), FieldValue::Text("Crash".into()));
    m.insert("urgent".into(), FieldValue::Bool(true));
    m
}

pub fn registry_with(
    ext: &ExtensionId,
    declared: &[Capability],
    granted: &[Capability],
) -> CapabilityRegistry {
    let mut r = CapabilityRegistry::new();
    r.declare(ext, declared.iter().cloned());
    for cap in granted {
        r.grant(ext, cap)
            .expect("fixture grants a declared, known capability");
    }
    r
}
