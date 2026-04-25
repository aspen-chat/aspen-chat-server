import QtQuick

// ListView tuned for a desktop pointer.
//
// Plain ``ListView`` inherits from ``Flickable``, whose default
// ``interactive: true`` mode treats any mouse press-and-drag as a
// touchscreen flick gesture. On a desktop that has two unwanted
// consequences:
//   * the user cannot drag to select text inside a delegate (the list
//     swallows the press as the start of a flick), and
//   * any incidental mouse movement during a click triggers the
//     inertial scroll, which feels alien with a real pointer.
//
// Disabling ``interactive`` removes the press-and-drag gesture but
// also disables the Flickable's built-in wheel handling, so we
// reinstate it via a ``WheelHandler``. The attached ``ScrollBar``
// (configured by callers) remains draggable for users who prefer
// scrollbar-driven navigation.
ListView {
    id: list

    interactive: false
    boundsBehavior: Flickable.StopAtBounds

    // Pixels-per-wheel-notch. ``angleDelta.y`` is in eighths of a
    // degree, with 120 units per standard wheel notch; dividing by
    // 1.5 lands close to the platform Flickable's per-notch step
    // without overshoot or rubber-band, and keeps high-resolution
    // (smooth) scroll wheels smooth because the formula is linear.
    property real wheelStepDivisor: 1.5

    WheelHandler {
        onWheel: (event) => {
            const dy = event.angleDelta.y / list.wheelStepDivisor
            const maxY = list.originY
                + Math.max(0, list.contentHeight - list.height)
            list.contentY = Math.max(
                list.originY,
                Math.min(maxY, list.contentY - dy))
        }
    }
}
