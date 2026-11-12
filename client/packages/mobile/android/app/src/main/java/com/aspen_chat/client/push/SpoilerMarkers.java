package com.aspen_chat.client.push;

/** Whether a message's text may hold a spoiler, for the notification code's own wording. */
final class SpoilerMarkers {

    private SpoilerMarkers() {}

    /**
     * Whether {@code content} holds a spoiler's marker anywhere: {@code ||}, {@code >!}, or
     * {@code !<}, or {@code \|\|}, which is how a table's cell writes the first. No spoiler is
     * written without one. Which of them make a spoiler is the client's Markdown to say
     * (spec/spoiler_vectors.json), and nothing here reads Markdown, so text holding one is not
     * shown at all: that shows nothing a spoiler hides, at the cost of hiding some texts
     * that only look as if they held one (a table, a shell pipeline).
     */
    static boolean present(String content) {
        return content.contains("||") || content.contains(">!") || content.contains("!<") || content.contains("\\|\\|");
    }
}
