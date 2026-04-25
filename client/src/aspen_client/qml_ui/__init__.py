"""Opt-in Qt Quick UI for the Aspen client.

The package mirrors the responsibilities of the Widgets-based UI but
expresses them as a QML scene tree backed by ``QObject`` controllers and
``QAbstractListModel`` subclasses. The non-UI layers
(:class:`AspenApiClient`, :class:`TaskSpawner`, :class:`ClientState`,
:class:`EventStreamClient`, :class:`IconCache`,
:class:`LinkPreviewImageCache`, :class:`UserDirectory`) are reused
unchanged; the only thing this layer adds is the model/controller bridge
plus the QML files themselves.

Reach the Quick UI by exporting ``ASPEN_UI=quick`` before invoking
``aspen-client``; the default remains the Widgets implementation in
:mod:`aspen_client.ui` until the Quick path reaches parity.
"""
