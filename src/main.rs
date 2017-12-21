#[macro_use] extern crate failure;
extern crate chrono;
extern crate gdk;
extern crate gtk;
extern crate rusqlite;
extern crate synac;
extern crate xdg;

mod connections;
mod functions;
mod messages;
mod typing;

use failure::Error;
use gdk::Screen;
use gtk::prelude::*;
use gtk::{
    Align,
    Box as GtkBox,
    Button,
    ButtonsType,
    CheckButton,
    CssProvider,
    Dialog,
    DialogFlags,
    Entry,
    EventBox,
    InputPurpose,
    Label,
    Menu,
    MenuItem,
    MessageDialog,
    MessageType,
    Orientation,
    PolicyType,
    PositionType,
    ResponseType,
    Revealer,
    RevealerTransitionType,
    ScrolledWindow,
    Separator,
    SeparatorMenuItem,
    Stack,
    StyleContext,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
    Window,
    WindowType
};
use connections::Connections;
use functions::*;
use rusqlite::Connection as SqlConnection;
use std::cell::RefCell;
use std::env;
use std::io::{Error as IoError, ErrorKind as IoErrorKind};
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use synac::common::{self, Packet};
use xdg::BaseDirectories;

#[derive(Debug, Fail)]
#[fail(display = "sadly GTK+ doesn't support unicode paths")]
struct UnicodePathError;

struct App {
    connections: Arc<Connections>,
    db: Rc<SqlConnection>,

    channel_name: Label,
    channels: GtkBox,
    message_edit: Revealer,
    message_edit_id: RefCell<Option<usize>>,
    message_edit_input: Entry,
    messages: GtkBox,
    messages_scroll: ScrolledWindow,
    server_name: Label,
    servers: GtkBox,
    stack: Stack,
    stack_edit_channel: GtkBox,
    stack_edit_server: GtkBox,
    stack_main: GtkBox,
    user_stack: Stack,
    user_stack_edit: Entry,
    user_stack_text: EventBox,
    typing: Label,
    window: Window
}
struct EditServer {
    name: Entry,
    server: Entry,
    hash: Entry
}
struct EditChannel {
    name: Entry,
    mode_bots: GtkBox,
    mode_users: GtkBox
}

fn main() {
    let basedirs = match BaseDirectories::with_prefix("synac") {
        Ok(basedirs) => basedirs,
        Err(err) => { eprintln!("error initializing xdg: {}", err); return; }
    };
    let path = match basedirs.find_data_file("data.sqlite") {
        Some(path) => path,
        None => match basedirs.place_data_file("data.sqlite") {
            Ok(path) => path,
            Err(err) => { eprintln!("error placing config: {}", err); return; }
        }
    };
    let db = match SqlConnection::open(&path) {
        Ok(ok) => ok,
        Err(err) => {
            eprintln!("Failed to open database");
            eprintln!("{}", err);
            return;
        }
    };
    db.execute("CREATE TABLE IF NOT EXISTS data (
                    key     TEXT NOT NULL UNIQUE,
                    value   TEXT NOT NULL
                )", &[])
        .expect("Couldn't create SQLite table");
    db.execute("CREATE TABLE IF NOT EXISTS servers (
                    ip      TEXT NOT NULL PRIMARY KEY,
                    name    TEXT NOT NULL,
                    hash    BLOB NOT NULL,
                    token   TEXT
                )", &[])
        .expect("Couldn't create SQLite table");

    let nick = {
        let mut stmt = db.prepare("SELECT value FROM data WHERE key = 'nick'").unwrap();
        let mut rows = stmt.query(&[]).unwrap();

        if let Some(row) = rows.next() {
            row.unwrap().get::<_, String>(0)
        } else {
            #[cfg(unix)]
            { env::var("USER").unwrap_or_else(|_| String::from("unknown")) }
            #[cfg(windows)]
            { env::var("USERNAME").unwrap_or_else(|_| String::from("unknown")) }
            #[cfg(not(any(unix, windows)))]
            { String::from("unknown") }
        }
    };

    if let Err(err) = gtk::init() {
        eprintln!("gtk error: {}", err);
        return;
    }

    let window = Window::new(WindowType::Toplevel);
    window.set_title("Synac GTK+ client");
    window.set_default_size(1000, 700);

    let app = Rc::new(App {
        channel_name: Label::new(""),
        channels: GtkBox::new(Orientation::Vertical, 2),
        connections: Connections::new(&db, nick),
        db: Rc::new(db),
        message_edit: Revealer::new(),
        message_edit_id: RefCell::new(None),
        message_edit_input: Entry::new(),
        messages: GtkBox::new(Orientation::Vertical, 10),
        messages_scroll: ScrolledWindow::new(None, None),
        server_name: Label::new(""),
        servers: GtkBox::new(Orientation::Vertical, 2),
        stack: Stack::new(),
        stack_edit_channel: GtkBox::new(Orientation::Vertical, 2),
        stack_edit_server: GtkBox::new(Orientation::Vertical, 2),
        stack_main: GtkBox::new(Orientation::Horizontal, 10),
        user_stack: Stack::new(),
        user_stack_edit: Entry::new(),
        user_stack_text: EventBox::new(),
        typing: Label::new(""),
        window: window
    });
    let edit_server = Rc::new(EditServer {
        name: Entry::new(),
        server: Entry::new(),
        hash: Entry::new()
    });
    let edit_channel = Rc::new(EditChannel {
        name: Entry::new(),
        mode_bots: GtkBox::new(Orientation::Vertical, 2),
        mode_users: GtkBox::new(Orientation::Vertical, 2)
    });

    app.stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    app.user_stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    app.stack.add(&app.stack_main);
    app.stack.add(&app.stack_edit_server);
    app.stack.add(&app.stack_edit_channel);

    app.user_stack.add(&app.user_stack_text);
    app.user_stack.add(&app.user_stack_edit);

    let user_name = Label::new(&**app.connections.nick.read().unwrap());
    add_class(&user_name, "bold");

    app.user_stack_edit.set_alignment(0.5);

    let app_clone = Rc::clone(&app);
    let user_name_clone = user_name.clone();
    app.user_stack_edit.connect_activate(move |input| {
        let text = input.get_text().unwrap_or_default();
        let old = app_clone.connections.nick.read().unwrap();
        app_clone.user_stack.set_visible_child(&app_clone.user_stack_text);
        if text.is_empty() || text == *old {
            return;
        }
        user_name_clone.set_text(&text);

        drop(old);

        app_clone.connections.foreach(|synac| {
            let result = synac.session.send(&Packet::LoginUpdate(common::LoginUpdate {
                name: Some(text.clone()),
                password_current: None,
                password_new: None,
                reset_token: false
            }));
            if let Err(err) = result {
                alert(&app_clone.window, MessageType::Warning, &err.to_string());
            }
        });

        *app_clone.connections.nick.write().unwrap() = text;
    });
    let app_clone = Rc::clone(&app);
    app.user_stack_edit.connect_focus_out_event(move |_, _| {
        app_clone.user_stack.set_visible_child(&app_clone.user_stack_text);
        Inhibit(false)
    });

    let servers_wrapper = GtkBox::new(Orientation::Vertical, 0);

    user_name.set_property_margin(10);

    app.user_stack_text.add(&user_name);

    let app_clone = Rc::clone(&app);
    app.user_stack_text.connect_button_press_event(move |_, event| {
        if event.get_button() == 1 {
            app_clone.user_stack_edit.set_text(&app_clone.connections.nick.read().unwrap());
            app_clone.user_stack_edit.grab_focus();
            app_clone.user_stack.set_visible_child(&app_clone.user_stack_edit);
        }
        Inhibit(false)
    });
    servers_wrapper.add(&app.user_stack);

    servers_wrapper.add(&Separator::new(Orientation::Vertical));

    render_servers(&app);
    servers_wrapper.add(&app.servers);

    let add = Button::new_with_label("Add...");
    add_class(&add, "add");
    add.set_valign(Align::End);
    add.set_vexpand(true);

    let app_clone = Rc::clone(&app);
    let edit_server_clone = Rc::clone(&edit_server);
    add.connect_clicked(move |_| {
        edit_server_clone.name.set_text("");
        edit_server_clone.server.set_text("");
        edit_server_clone.hash.set_text("");

        app_clone.stack.set_visible_child(&app_clone.stack_edit_server);
    });

    servers_wrapper.add(&add);

    app.stack_main.add(&servers_wrapper);

    app.stack_main.add(&Separator::new(Orientation::Horizontal));

    let channels_wrapper = GtkBox::new(Orientation::Vertical, 0);

    add_class(&app.server_name, "bold");
    app.server_name.set_property_margin(10);
    channels_wrapper.add(&app.server_name);

    channels_wrapper.add(&Separator::new(Orientation::Vertical));

    channels_wrapper.add(&app.channels);

    let add = Button::new_with_label("Add...");
    add_class(&add, "add");
    add.set_valign(Align::End);
    add.set_vexpand(true);

    let app_clone = Rc::clone(&app);
    let edit_server_clone = Rc::clone(&edit_server);
    add.connect_clicked(move |_| {
        edit_server_clone.name.set_text("");
        edit_server_clone.server.set_text("");
        edit_server_clone.hash.set_text("");

        app_clone.stack.set_visible_child(&app_clone.stack_edit_channel);
    });

    channels_wrapper.add(&add);
    app.stack_main.add(&channels_wrapper);

    app.stack_main.add(&Separator::new(Orientation::Horizontal));

    let content = GtkBox::new(Orientation::Vertical, 2);

    add_class(&app.channel_name, "bold");
    app.channel_name.set_property_margin(10);

    let app_clone = Rc::clone(&app);
    let edit_channel_clone = Rc::clone(&edit_channel);
    add.connect_clicked(move |_| {
        edit_channel_clone.name.set_text("");
        render_mode(&edit_channel_clone.mode_bots, 0);
        render_mode(&edit_channel_clone.mode_users, common::PERM_READ | common::PERM_WRITE);

        app_clone.stack.set_visible_child(&app_clone.stack_edit_channel);
    });

    content.add(&app.channel_name);
    content.add(&Separator::new(Orientation::Vertical));

    app.messages.set_valign(Align::End);
    app.messages.set_vexpand(true);
    app.messages_scroll.add(&app.messages);

    app.messages_scroll.set_policy(PolicyType::Never, PolicyType::Always);

    app.messages_scroll.get_vadjustment().unwrap().connect_changed(move |vadjustment| {
        let upper = vadjustment.get_upper() - vadjustment.get_page_size();
        if vadjustment.get_value() + 100.0 >= upper {
            vadjustment.set_value(upper);
        }
    });
    let app_clone = Rc::clone(&app);
    app.messages_scroll.connect_edge_reached(move |_, pos| {
        if pos != PositionType::Top {
            return;
        }
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if let Ok(synac) = result {
                    if let Some(channel) = synac.current_channel {
                        if let Err(err) = synac.session.send(&Packet::MessageList(common::MessageList {
                            after: None,
                            before: synac.messages.get(channel).first().map(|msg| msg.id),
                            channel: channel,
                            limit: common::LIMIT_BULK
                        })) {
                            eprintln!("error sending packet: {}", err);
                        }
                    }
                }
            });
        }
    });
    content.add(&app.messages_scroll);

    app.message_edit.set_transition_type(RevealerTransitionType::SlideUp);

    let message_edit = GtkBox::new(Orientation::Vertical, 2);

    message_edit.add(&Label::new("Edit message"));

    let app_clone = Rc::clone(&app);
    app.message_edit_input.connect_activate(move |input| {
        let text = input.get_text().unwrap_or_default();
        if text.is_empty() {
            return;
        }
        input.set_sensitive(false);
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if result.is_err() {
                    return;
                }
                let synac = result.unwrap();

                if let Err(err) = synac.session.send(&Packet::MessageUpdate(common::MessageUpdate {
                    id: app_clone.message_edit_id.borrow().expect("wait how is this variable not set"),
                    text: text.into_bytes()
                })) {
                    eprintln!("failed to send packet: {}", err);
                }
            });
        }
        input.set_sensitive(true);
        app_clone.message_edit.set_reveal_child(false);
    });

    message_edit.add(&app.message_edit_input);

    let message_edit_cancel = Button::new_with_label("Cancel");
    let app_clone = Rc::clone(&app);
    message_edit_cancel.connect_clicked(move |_| {
        app_clone.message_edit.set_reveal_child(false);
    });
    message_edit.add(&message_edit_cancel);

    app.message_edit.add(&message_edit);
    content.add(&app.message_edit);

    let input = Entry::new();
    input.set_hexpand(true);
    input.set_placeholder_text("Send a message...");

    let typing_duration = Duration::from_secs(common::TYPING_TIMEOUT as u64 / 2); // TODO: const fn
    let typing_last = RefCell::new(Instant::now());

    let app_clone = Rc::clone(&app);
    input.connect_property_text_notify(move |_| {
        let mut typing_last = typing_last.borrow_mut();
        if typing_last.elapsed() < typing_duration {
            return;
        }
        *typing_last = Instant::now();

        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if let Ok(synac) = result {
                    if let Some(channel) = synac.current_channel {
                        if let Err(err) = synac.session.send(&Packet::Typing(common::Typing {
                            channel: channel
                        })) {
                            eprintln!("failed to send packet: {}", err);
                        }
                    }
                }
            });
        }
    });
    let app_clone = Rc::clone(&app);
    input.connect_activate(move |input| {
        let text = input.get_text().unwrap_or_default();
        if text.is_empty() {
            return;
        }
        input.set_sensitive(false);
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if result.is_err() {
                    return;
                }
                let synac = result.unwrap();
                if synac.current_channel.is_none() {
                    return;
                }
                let channel = synac.current_channel.unwrap();
                if let Err(err) = synac.session.send(&Packet::MessageCreate(common::MessageCreate {
                    channel: channel,
                    text: text.into_bytes()
                })) {
                    if let Ok(io_err) = err.downcast::<IoError>() {
                        if io_err.kind() != IoErrorKind::BrokenPipe {
                            return;
                        }
                    }

                    let mut stmt = app_clone.db.prepare("SELECT hash, token FROM servers WHERE ip = ?").unwrap();
                    let mut rows = stmt.query(&[&addr.to_string()]).unwrap();

                    if let Some(row) = rows.next() {
                        let row = row.unwrap();

                        let hash = row.get(0);
                        let token = row.get(1);

                        connect(&app_clone, addr, hash, token);
                    }
                }
            });
        }
        input.set_text("");
        input.set_sensitive(true);
        input.grab_focus();
    });

    content.add(&input);

    app.typing.set_xalign(0.0);
    content.add(&app.typing);

    app.stack_main.add(&content);

    app.stack_edit_server.set_property_margin(10);

    edit_server.name.set_placeholder_text("Server name...");
    app.stack_edit_server.add(&edit_server.name);
    app.stack_edit_server.add(&Label::new("The server name. This can be anything you want it to."));

    edit_server.server.set_placeholder_text("Server IP...");
    app.stack_edit_server.add(&edit_server.server);
    app.stack_edit_server.add(&Label::new(&*format!("The server IP address. The default port is {}.", common::DEFAULT_PORT)));

    edit_server.hash.set_placeholder_text("Server's certificate hash...");
    app.stack_edit_server.add(&edit_server.hash);
    app.stack_edit_server.add(&Label::new("The server's certificate public key hash.\n\
                               This is to verify nobody is snooping on your connection"));

    let edit_server_controls = GtkBox::new(Orientation::Horizontal, 2);

    let edit_server_cancel = Button::new_with_label("Cancel");
    let app_clone = Rc::clone(&app);
    edit_server_cancel.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_main);
    });
    edit_server_controls.add(&edit_server_cancel);

    let edit_server_ok = Button::new_with_label("Ok");

    let app_clone = Rc::clone(&app);
    edit_server_ok.connect_clicked(move |_| {
        let name_text   = edit_server.name.get_text().unwrap_or_default();
        let server_text = edit_server.server.get_text().unwrap_or_default();
        let hash_text   = edit_server.hash.get_text().unwrap_or_default();

        let addr = match connections::parse_addr(&server_text) {
            Some(addr) => addr,
            None => return
        };

        app_clone.stack.set_visible_child(&app_clone.stack_main);

        app_clone.db.execute(
            "INSERT INTO servers (name, ip, hash) VALUES (?, ?, ?)",
            &[&name_text, &addr.to_string(), &hash_text]
        ).unwrap();
        render_servers(&app_clone);
    });

    edit_server_controls.add(&edit_server_ok);
    app.stack_edit_server.add(&edit_server_controls);

    app.stack_edit_channel.set_property_margin(10);

    edit_channel.name.set_placeholder_text("Channel name...");
    app.stack_edit_channel.add(&edit_channel.name);

    app.stack_edit_channel.add(&Label::new("The channel name."));

    let label = Label::new("Default permissions for bots: ");
    label.set_xalign(0.0);
    app.stack_edit_channel.add(&label);

    app.stack_edit_channel.add(&edit_channel.mode_bots);

    let label = Label::new("Default permissions for users: ");
    label.set_xalign(0.0);
    app.stack_edit_channel.add(&label);

    app.stack_edit_channel.add(&edit_channel.mode_users);

    let edit_channel_controls = GtkBox::new(Orientation::Horizontal, 2);

    let edit_channel_cancel = Button::new_with_label("Cancel");
    let app_clone = Rc::clone(&app);
    edit_channel_cancel.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_main);
    });
    edit_channel_controls.add(&edit_channel_cancel);

    let edit_channel_ok = Button::new_with_label("Ok");

    let app_clone = Rc::clone(&app);
    edit_channel_ok.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_main);
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if result.is_err() { return; }
                let synac = result.unwrap();

                let name = edit_channel.name.get_text().unwrap_or_default();

                if name.is_empty() {
                    return;
                }

                let result = synac.session.send(&Packet::ChannelCreate(common::ChannelCreate {
                    default_mode_bot: get_mode(&edit_channel.mode_bots).unwrap(),
                    default_mode_user: get_mode(&edit_channel.mode_users).unwrap(),
                    name: name
                }));
                if let Err(err) = result {
                    alert(&app_clone.window, MessageType::Error, &err.to_string());
                }
            });
        }
    });

    edit_channel_controls.add(&edit_channel_ok);
    app.stack_edit_channel.add(&edit_channel_controls);

    app.window.add(&app.stack);

    // Load CSS
    let screen = Screen::get_default();
    match screen {
        None => eprintln!("error: no default screen"),
        Some(screen) => {
            let css = CssProvider::new();
            let result: Result<(), Error> = if let Some(file) = basedirs.find_config_file("style.css") {
                if let Some(s) = file.to_str() {
                    css.load_from_path(s).map_err(Error::from)
                } else {
                    Err(UnicodePathError.into())
                }
            } else {
                let dark = if let Some(settings) = app.window.get_settings() {
                    settings.get_property_gtk_application_prefer_dark_theme()
                } else { false };

                css.load_from_data(if dark {
                    include_bytes!("dark.css")
                } else {
                    include_bytes!("light.css")
                }).map_err(Error::from)
            };
            if let Err(err) = result {
                alert(&app.window, MessageType::Error, &err.to_string());
            }
            StyleContext::add_provider_for_screen(&screen, &css, STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    }

    app.window.show_all();
    app.window.connect_delete_event(|_, _| {
        gtk::main_quit();
        Inhibit(false)
    });

    gtk::idle_add(move || {
        let mut channels = false;
        let mut messages = false;
        let mut addr = None;

        let current_server = *app.connections.current_server.lock().unwrap();

        if let Err(err) = app.connections.try_read(|synac, packet| {
            println!("received {:?}", packet);
            if current_server != Some(synac.addr) {
                return;
            }
            addr = Some(synac.addr);
            match packet {
                Packet::ChannelReceive(_)       => channels = true,
                Packet::ChannelDeleteReceive(_) => channels = true,
                Packet::MessageReceive(_)       => messages = true,
                Packet::MessageDeleteReceive(_) => messages = true,
                _ => {}
            }
        }) {
            eprintln!("receive error: {}", err);
            return Continue(true);
        }

        if let Some(addr) = current_server {
            app.connections.execute(addr, |result| {
                if let Ok(synac) = result {
                    if let Some(typing) = synac.typing.check(synac.current_channel, &synac.state) {
                        app.typing.set_text(&typing);
                    }
                }
            });
        }

        if let Some(addr) = addr {
            if channels {
                render_channels(Some(addr), &app);
            } else if messages {
                render_messages(Some(addr), &app);
            }
        }

        Continue(true)
    });
    gtk::main();
}
