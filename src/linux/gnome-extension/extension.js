import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import {popupPosition} from './geometry.js';
import {editorWindowIndex} from './activation.js';

export default class MonitorPanel extends Extension {
    enable() {
        this._button = new PanelMenu.Button(0.5, 'AI Agent Monitor', true);
        const box = new St.BoxLayout({style_class: 'panel-status-menu-box'});
        this._art = new St.Widget({layout_manager: new Clutter.BinLayout(), width: 22, height: 22, y_align: Clutter.ActorAlign.CENTER});
        this._baseIcon = this._icon('ai-agent-monitor-symbolic.svg');
        this._art.add_child(this._baseIcon);
        this._leftEye = this._icon('ai-agent-monitor-eyes-left-symbolic.svg');
        this._rightEye = this._icon('ai-agent-monitor-eyes-right-symbolic.svg');
        for (const eye of [this._leftEye, this._rightEye]) {
            eye.set_style('icon-size: 22px; color: #33d17a;');
            this._art.add_child(eye);
            eye.hide();
        }
        box.add_child(this._art);
        this._workingCount = new St.Label({y_align: Clutter.ActorAlign.CENTER, style: 'color: #33d17a; font-size: 11px; font-weight: normal;'});
        this._finishedCount = this._badge('#73c7ff', '#122534');
        this._pendingCount = this._badge('#ff9f0a', '#202020');
        for (const count of [this._workingCount, this._finishedCount, this._pendingCount])
            box.add_child(count);
        box.set_style('spacing: 4px;');
        this._phase = 0;
        this._eyeTimer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 140, () => {
            this._animateEyes();
            return GLib.SOURCE_CONTINUE;
        });
        this._acceleratorId = global.display.connect('accelerator-activated', (_display, action) => {
            if (action === this._accelerator)
                this._toggle();
        });
        this._button.add_child(box);
        this._button.connect('button-press-event', (_actor, event) => {
            if (event.get_button() === Clutter.BUTTON_PRIMARY) {
                this._toggle();
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
        this._button.connect('key-press-event', (_actor, event) => {
            if ([Clutter.KEY_Return, Clutter.KEY_space].includes(event.get_key_symbol())) {
                this._toggle();
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
        Main.panel.addToStatusArea(this.uuid, this._button, 0, 'right');
        this._button.hide();
        this._activation = Gio.DBusExportedObject.wrapJSObject(
            '<node><interface name="io.github.melsomino.AIAgentMonitor.Panel">' +
            '<method name="ActivateEditor"><arg type="s" direction="in"/>' +
            '<arg type="u" direction="in"/><arg type="b" direction="out"/>' +
            '</method></interface></node>', this);
        this._activation.export(Gio.DBus.session, '/io/github/melsomino/AIAgentMonitor/Panel');
        // The Rust window detects this name and enables panel-specific behavior.
        this._panelName = Gio.bus_own_name(Gio.BusType.SESSION,
            'io.github.melsomino.AIAgentMonitor.Panel', Gio.BusNameOwnerFlags.NONE,
            null, null, null);
        this._watch = Gio.bus_watch_name(Gio.BusType.SESSION,
            'io.github.melsomino.AIAgentMonitor', Gio.BusNameWatcherFlags.NONE,
            (connection, _name, owner) => this._connect(connection, owner),
            () => this._disconnect());
        this._mapId = global.window_manager.connect('map', (_wm, actor) => {
            const window = actor.meta_window;
            if (this._isPanel(window)) {
                // Positioning is performed by the compositor, including on Wayland.
                this._position(window);
                window.activate(global.get_current_time());
            }
        });
    }

    _connect(connection, owner) {
        this._disconnect();
        const cancellable = new Gio.Cancellable();
        this._cancellable = cancellable;
        connection.call('org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', 'GetConnectionUnixProcessID',
            new GLib.Variant('(s)', [owner]), new GLib.VariantType('(u)'),
            Gio.DBusCallFlags.NONE, 3000, cancellable, (source, response) => {
                try {
                    const result = source.call_finish(response);
                    if (cancellable.is_cancelled())
                        return;
                    this._pid = result.deep_unpack()[0];
                    this._createProxy(connection, owner, cancellable);
                } catch (error) {
                    if (!cancellable.is_cancelled())
                        console.error(`AI Agent Monitor: ${error.message}`);
                }
            });
    }

    ActivateEditorAsync([target, timestamp], invocation) {
        if (invocation.get_sender() !== this._proxy?.get_name_owner()) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied',
                'Only the connected monitor may activate a session window');
            return;
        }
        const windows = global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null);
        const tracker = Shell.WindowTracker.get_default();
        const index = editorWindowIndex(target, windows.map(window => ({
            appId: tracker.get_window_app(window)?.get_id(),
            wmClass: window.get_wm_class(), title: window.get_title(),
        })));
        if (index >= 0)
            Main.activateWindow(windows[index], timestamp || global.get_current_time());
        invocation.return_value(new GLib.Variant('(b)', [index >= 0]));
    }

    _createProxy(connection, owner, cancellable) {
        Gio.DBusProxy.new(connection, Gio.DBusProxyFlags.NONE, null, owner,
            '/io/github/melsomino/AIAgentMonitor', 'io.github.melsomino.AIAgentMonitor',
            cancellable, (_source, response) => {
                try {
                    const proxy = Gio.DBusProxy.new_finish(response);
                    if (cancellable.is_cancelled())
                        return;
                    this._proxy = proxy;
                    this._propertiesId = proxy.connect('g-properties-changed', () => this._update());
                    this._button.show();
                    this._update();
                    for (const actor of global.get_window_actors()) {
                        if (this._isPanel(actor.meta_window))
                            this._position(actor.meta_window);
                    }
                } catch (error) {
                    if (!cancellable.is_cancelled())
                        console.error(`AI Agent Monitor: ${error.message}`);
                }
            });
    }

    _disconnect() {
        if (this._accelerator)
            global.display.ungrab_accelerator(this._accelerator);
        this._accelerator = 0;
        this._shortcut = '';
        this._working = false;
        this._cancellable?.cancel();
        this._cancellable = null;
        if (this._proxy && this._propertiesId)
            this._proxy.disconnect(this._propertiesId);
        this._propertiesId = 0;
        this._proxy = null;
        this._pid = null;
        this._button?.remove_style_pseudo_class('checked');
        this._button?.hide();
    }

    _icon(filename) {
        return new St.Icon({gicon: new Gio.FileIcon({file: this.dir.get_child(filename)}),
            icon_size: 22, width: 22, height: 22,
            x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER,
            // St.Icon uses a square texture. The SVG's transparent square canvas
            // preserves the original 30:22 artwork instead of stretching it.
            style_class: 'monitor-panel-icon', style: 'icon-size: 22px;'});
    }

    _badge(background, foreground) {
        return new St.Bin({
            x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER,
            style: `background-color: ${background}; border-radius: 999px; padding: 0;`,
            child: new St.Label({
                x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER,
                style: `color: ${foreground}; font-size: 10px; font-weight: normal; padding: 0;`,
            }),
        });
    }

    _update() {
        const count = name => this._proxy?.get_cached_property(name)?.deep_unpack() ?? 0;
        for (const [widget, value] of [[this._workingCount, count('Working')],
            [this._finishedCount, count('Finished')], [this._pendingCount, count('Pending')]]) {
            if (widget === this._workingCount) {
                widget.text = String(value);
            } else {
                const label = widget.get_child();
                label.text = String(value);
                const diameter = Math.ceil(Math.max(18,
                    label.get_preferred_width(-1)[1] + 8,
                    label.get_preferred_height(-1)[1] + 4));
                widget.set_size(diameter, diameter);
            }
            widget.visible = value > 0;
        }
        this._working = count('Working') > 0;
        this._animation = this._proxy?.get_cached_property('Animation')?.deep_unpack() ?? true;
        this._baseIcon.gicon = new Gio.FileIcon({file: this.dir.get_child(
            this._working ? 'ai-agent-monitor-outline-symbolic.svg' : 'ai-agent-monitor-symbolic.svg')});
        if (count('Visible'))
            this._button.add_style_pseudo_class('checked');
        else
            this._button.remove_style_pseudo_class('checked');
        this._button.set_accessible_name(`AI Agent Monitor, ${count('Working')} working, ${count('Finished')} finished, ${count('Pending')} requests`);
        this._animateEyes(false);
        const shortcut = this._proxy?.get_cached_property('Shortcut')?.deep_unpack() ?? '';
        if (shortcut !== this._shortcut) {
            if (this._accelerator)
                global.display.ungrab_accelerator(this._accelerator);
            this._shortcut = shortcut;
            this._accelerator = shortcut ? global.display.grab_accelerator(shortcut, Meta.KeyBindingFlags.NONE) : 0;
            if (this._accelerator) {
                Main.wm.allowKeybinding(Meta.external_binding_name_for_action(this._accelerator), Shell.ActionMode.NORMAL);
            } else if (shortcut) {
                this._proxy.call('ShortcutFailed', new GLib.Variant('(s)', ['This shortcut is already in use. Choose another combination.']),
                    Gio.DBusCallFlags.NONE, 3000, this._cancellable, null);
            }
        }
    }

    _animateEyes(advance = true) {
        const steps = [[0.5, 0], [1, 0], [1, 0.5], [1, 1], [0.5, 1], [0, 1], [0, 0.5], [0, 0]];
        const levels = this._animation ? steps[this._phase] : [1, 1];
        this._leftEye.visible = this._rightEye.visible = !!this._working;
        if (this._working) {
            this._leftEye.opacity = Math.round(255 * levels[0]);
            this._rightEye.opacity = Math.round(255 * levels[1]);
            if (advance && this._animation)
                this._phase = (this._phase + 1) % steps.length;
        }
    }

    _isPanel(window) {
        return window && window.get_pid() === this._pid && window.get_title() === 'AI Agent Monitor';
    }

    _toggle() {
        if (!this._proxy)
            return;
        this._proxy.call('Toggle', null, Gio.DBusCallFlags.NONE, 3000,
            this._cancellable, (proxy, result) => {
                try {
                    proxy.call_finish(result);
                    for (const actor of global.get_window_actors()) {
                        if (this._isPanel(actor.meta_window))
                            this._position(actor.meta_window);
                    }
                } catch (error) {
                    if (!this._cancellable?.is_cancelled())
                        console.error(`AI Agent Monitor: ${error.message}`);
                }
            });
    }

    _position(window) {
        const [x, y] = this._button.get_transformed_position();
        const [width, height] = this._button.get_transformed_size();
        const monitor = Main.layoutManager.findMonitorForActor(this._button);
        if (!monitor)
            return;
        const area = Main.layoutManager.getWorkAreaForMonitor(monitor.index);
        const frame = window.get_frame_rect();
        const [left, top] = popupPosition({x, y, width, height}, frame, area);
        window.move_frame(false, left, top);
    }

    disable() {
        if (this._eyeTimer)
            GLib.source_remove(this._eyeTimer);
        if (this._accelerator)
            global.display.ungrab_accelerator(this._accelerator);
        this._accelerator = 0;
        global.display.disconnect(this._acceleratorId);
        Gio.bus_unwatch_name(this._watch);
        Gio.bus_unown_name(this._panelName);
        this._activation.unexport();
        this._activation = null;
        global.window_manager.disconnect(this._mapId);
        this._disconnect();
        this._button.destroy();
        this._button = null;
    }
}
