// Stand-in for resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js,
// which only loads inside the org.gnome.Shell.Extensions service.
export class ExtensionPreferences {
    constructor(metadata) {
        this.metadata = metadata;
    }

    get uuid() {
        return this.metadata.uuid;
    }

    get path() {
        return this.metadata.path;
    }
}
