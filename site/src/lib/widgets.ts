// The widget set, as the crate has it. Each blurb is the widget module's own
// first line in `denise-ui/src/widgets/`, so this page cannot drift into
// describing something the toolkit does not do.

export type Widget = {
	id: string;
	name: string;
	group: string;
	file: string;
	blurb: string;
	more?: string;
};

export const groups = ['Text and input', 'Structure and navigation', 'Data and feedback', 'Pictures and media'];

export const widgets: Widget[] = [
	{ id: 'label', name: 'Label', group: 'Text and input', file: 'label.rs', blurb: 'Static text.', more: 'Not interactive, so a click falls through it to whatever is underneath — which is how a table row is assembled.' },
	{ id: 'button', name: 'Button', group: 'Text and input', file: 'button.rs', blurb: 'A pressable, focusable, message-emitting rectangle.', more: 'It holds a value of your type and emits it when pressed. No callbacks, anywhere.' },
	{ id: 'text-input', name: 'TextInput', group: 'Text and input', file: 'text_input.rs', blurb: 'A single-line editable text field.', more: 'Caret, placeholder, submit on Enter, dead keys, and selection: focus takes the whole field, a press places the caret, a second takes the word, a third takes everything. Deliberately absent for now: the clipboard and keyboard word motion.' },
	{ id: 'text-area', name: 'TextArea', group: 'Text and input', file: 'text_area.rs', blurb: 'A multi-line text editor that edits through a document it does not own.', more: 'It asks only for the lines it is about to paint, so a file too big to load can sit behind it. Selection — a press, a word, a line — undo, paging, go-to-line and match highlighting.' },
	{ id: 'checkbox', name: 'Checkbox', group: 'Text and input', file: 'checkbox.rs', blurb: 'A box, a tick, and a boolean.' },
	{ id: 'toggle', name: 'Toggle', group: 'Text and input', file: 'toggle.rs', blurb: 'The same boolean as a checkbox, with a different affordance.' },
	{ id: 'radio-group', name: 'RadioGroup', group: 'Text and input', file: 'radio.rs', blurb: 'One choice from a few, and the keyboard rules that make it one.' },
	{ id: 'slider', name: 'Slider', group: 'Text and input', file: 'slider.rs', blurb: 'A value in a range, dragged or typed.' },
	{ id: 'select', name: 'Select', group: 'Text and input', file: 'select.rs', blurb: 'The closed half of a dropdown, and the four lines that open it.', more: 'The open half is a popup: a scene above the tree, which is why it can overhang everything.' },
	{ id: 'rating', name: 'Rating', group: 'Text and input', file: 'rating.rs', blurb: 'Stars, filled to a value.' },

	{ id: 'panel', name: 'Panel', group: 'Structure and navigation', file: 'panel.rs', blurb: 'A themed rectangle: the background every other widget sits on.' },
	{ id: 'tabs', name: 'Tabs', group: 'Structure and navigation', file: 'tabs.rs', blurb: 'A row of labels where one is selected.', more: 'With events it becomes a document strip: close buttons, drag to reorder, double click to rename, a right-click menu and per-tab colours.' },
	{ id: 'menu-bar', name: 'MenuBar', group: 'Structure and navigation', file: 'menu.rs', blurb: 'A menu bar, and the popup a menu — or a right-click — opens as.' },
	{ id: 'list', name: 'List', group: 'Structure and navigation', file: 'list.rs', blurb: 'A vertical list of rows, one of them selected.', more: 'A selection below the fold scrolls itself into view, because scrolling is the tree’s job.' },
	{ id: 'tree', name: 'Tree', group: 'Structure and navigation', file: 'tree.rs', blurb: 'Rows at a depth, with a disclosure triangle and a hierarchy.' },
	{ id: 'collapse', name: 'Collapse', group: 'Structure and navigation', file: 'collapse.rs', blurb: 'A section that folds to its header, and the Accordion that makes several behave as one.', more: 'The fold animates through the same relayout path everything else uses, so stacked siblings move with it.' },
	{ id: 'divider', name: 'Divider', group: 'Structure and navigation', file: 'divider.rs', blurb: 'A line, optionally with a label in the middle.' },

	{ id: 'table', name: 'Table', group: 'Data and feedback', file: 'table.rs', blurb: 'Columns of cells under a pinned header.' },
	{ id: 'timeline', name: 'Timeline', group: 'Data and feedback', file: 'timeline.rs', blurb: 'Events in order: a time, a disc, a connector, a label.' },
	{ id: 'progress', name: 'Progress', group: 'Data and feedback', file: 'progress.rs', blurb: 'A track and a fill. The one widget here that is purely an output.' },
	{ id: 'radial-progress', name: 'RadialProgress', group: 'Data and feedback', file: 'radial.rs', blurb: 'A ring that fills clockwise, with room for a number in the middle.' },
	{ id: 'spinner', name: 'Spinner', group: 'Data and feedback', file: 'spinner.rs', blurb: 'A rotating arc, for when there is nothing to report but that something is happening.', more: 'It says that it is moving; the tree decides how often, through Ui::set_motion.' },
	{ id: 'badge', name: 'Badge', group: 'Data and feedback', file: 'badge.rs', blurb: 'A short string in a coloured pill.' },
	{ id: 'alert', name: 'Alert', group: 'Data and feedback', file: 'alert.rs', blurb: 'A coloured banner with a message.' },
	{ id: 'avatar', name: 'Avatar', group: 'Data and feedback', file: 'avatar.rs', blurb: 'A person, as a picture or as their initials.' },

	{ id: 'image', name: 'Image', group: 'Pictures and media', file: 'image.rs', blurb: 'A picture in a rectangle.', more: 'PNG, JPEG, GIF and BMP decode through denise-image into premultiplied pixels.' },
	{ id: 'carousel', name: 'Carousel', group: 'Pictures and media', file: 'carousel.rs', blurb: 'Pictures shown one at a time, sliding between them.' },
	{ id: 'video', name: 'Video', group: 'Pictures and media', file: 'video.rs', blurb: 'The rectangle a video plane sits in.', more: 'denise-video decodes through V4L2 onto a DRM plane, zero-copy, so the frames never pass through the rasteriser.' }
];

/** What the tree owns, so that a widget can stay a small thing. */
export const treeOwned = [
	{ name: 'Modal scenes', what: 'A modal is a scene pushed over a dimmed backdrop in the same buffer. Nothing underneath needs disabling: it simply stops receiving input.' },
	{ name: 'Drawers', what: 'A panel that slides in from an edge and dims what is behind it. Escape, or a press on the dim, slides it out.' },
	{ name: 'Shelves', what: 'Neither modal nor dimming — which is what lets the on-screen keyboard type into the field above it without the field ever losing its caret.' },
	{ name: 'Tooltips', what: 'Owned by the tree so they paint above every widget and below the pointer.' },
	{ name: 'Toasts', what: 'Transient messages that fade on the tree’s own clock.' },
	{ name: 'Popups', what: 'What a Select or a menu opens into, above everything, closed by Escape or a press outside.' },
	{ name: 'Scrolling', what: 'Mark a node scrollable and it becomes a viewport: wheel, page keys, touch-drag, clipping and hit testing all agree, because one reflow computes them.' },
	{ name: 'Focus and tab order', what: 'Tab moves through the tree in file order, and focus below the fold scrolls itself into view.' },
	{ name: 'The cursor sprite', what: 'On a bare panel there is nothing else to draw a pointer, so the tree composites one — and moving it costs two sprite-sized rectangles.' }
];
