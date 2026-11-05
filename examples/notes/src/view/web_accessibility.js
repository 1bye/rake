// GPUI Web 0.3.11 has one IME textarea and no AccessKit adapter. Expose the
// unfocused Notes fields here and give the live textarea the focused field's
// semantics. GPUI still owns all input events, selection, and text.
export class BrowserFields {
    constructor(search, title, body, focusField) {
        // shortcut: the pinned GPUI Web backend has no public IME element hook.
        this.mirror = Array.from(document.body.children).find(element =>
            element instanceof HTMLTextAreaElement &&
            element.style.position === "fixed" &&
            element.style.width === "1px" &&
            element.style.opacity === "0");
        if (!this.mirror) throw new Error("Fieldnotes could not find the GPUI keyboard receiver");

        this.original = new Map(["id", "aria-label", "aria-multiline"].map(
            name => [name, this.mirror.getAttribute(name)]
        ));
        this.mirror.id ||= "fieldnotes-keyboard";
        this.group = document.createElement("div");
        this.group.setAttribute("role", "group");
        this.group.setAttribute("aria-label", "Note fields");
        this.group.hidden = true;
        Object.assign(this.group.style, {
            position: "fixed", width: "1px", height: "1px", overflow: "hidden",
            clipPath: "inset(50%)", pointerEvents: "none",
        });

        this.fields = [search, title, body].map((name, index) => {
            const multiline = index === 2;
            const element = document.createElement(multiline ? "textarea" : "input");
            element.id = `fieldnotes-field-${index}`;
            element.tabIndex = -1;
            element.setAttribute("aria-label", name);
            element.addEventListener("focus", () => {
                if (!focusField(index)) return;
                this.labelMirror({name, multiline});
                this.mirror.focus({preventScroll: true});
            });
            this.group.append(element);
            return {element, name, multiline};
        });
        document.body.append(this.group);
    }

    labelMirror(field) {
        this.mirror.setAttribute("aria-label", field?.name ?? "Fieldnotes keyboard");
        if (field) {
            this.mirror.setAttribute("aria-multiline", String(field.multiline));
        } else {
            this.mirror.removeAttribute("aria-multiline");
        }
    }

    sync(ready, hasNote, active) {
        if (!ready || (!hasNote && active !== 0)) active = undefined;
        if (ready === this.ready && hasNote === this.hasNote && active === this.active) return;
        this.ready = ready;
        this.hasNote = hasNote;
        this.active = active;

        this.labelMirror(this.fields[active]);
        this.group.hidden = !ready;
        const order = [];
        for (const [index, field] of this.fields.entries()) {
            const available = ready && (index === 0 || hasNote);
            field.element.hidden = !available || index === active;
            if (available) order.push(index === active ? this.mirror.id : field.element.id);
        }
        // Set the reading order without moving the focused IME node in the DOM.
        this.group.setAttribute("aria-owns", order.join(" "));
    }

    remove() {
        this.group.remove();
        for (const [name, value] of this.original) {
            if (value === null) this.mirror.removeAttribute(name);
            else this.mirror.setAttribute(name, value);
        }
    }
}
