/**
 * WebSocket Port Adapter for capnpc-elm3
 * 
 * This module provides the JavaScript side of the Elm/JS interop for WebSocket
 * communication. It handles:
 * - WebSocket connection management
 * - Length-prefixed message framing
 * - Binary message encoding/decoding
 * 
 * Reference: capnp-typescript/src/rpc/websocket-transport.ts
 */

/**
 * Create a WebSocket manager for use with Elm ports
 * 
 * @param {Object} ports - Elm app ports object
 * @param {Object} options - Configuration options
 * @param {number} options.connectTimeoutMs - Connection timeout in milliseconds
 * @param {string} options.binaryType - Binary type ('arraybuffer' or 'blob')
 * @returns {Object} WebSocket manager with connect/disconnect methods
 */
export function createWebSocketManager(ports, options = {}) {
    const {
        connectTimeoutMs = 10000,
        binaryType = 'arraybuffer'
    } = options;

    let ws = null;
    let pendingBuffer = null;
    let pendingLength = -1;

    /**
     * Connect to a WebSocket server
     * @param {string} url - WebSocket URL
     */
    function connect(url) {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.close();
        }

        ws = new WebSocket(url);
        ws.binaryType = binaryType;

        // Connection timeout
        const timeoutId = setTimeout(() => {
            if (ws && ws.readyState === WebSocket.CONNECTING) {
                ws.close();
                ports.wsConnectionState.send({
                    state: 'error',
                    error: 'Connection timeout'
                });
            }
        }, connectTimeoutMs);

        ws.onopen = () => {
            clearTimeout(timeoutId);
            ports.wsConnectionState.send({
                state: 'connected',
                error: null
            });
        };

        ws.onclose = (event) => {
            clearTimeout(timeoutId);
            ports.wsConnectionState.send({
                state: 'disconnected',
                error: event.reason || null
            });
            ws = null;
        };

        ws.onerror = (_event) => {
            clearTimeout(timeoutId);
            ports.wsConnectionState.send({
                state: 'error',
                error: 'WebSocket error'
            });
        };

        ws.onmessage = (event) => {
            handleMessage(event.data);
        };
    }

    /**
     * Disconnect from the WebSocket server
     */
    function disconnect() {
        if (ws) {
            ws.close();
            ws = null;
        }
        pendingBuffer = null;
        pendingLength = -1;
    }

    /**
     * Send framed binary data through WebSocket
     * @param {Object} payload - { data: number[] }
     */
    function send(payload) {
        if (!ws || ws.readyState !== WebSocket.OPEN) {
            console.error('WebSocket not connected');
            return;
        }

        const messageData = new Uint8Array(payload.data);
        const frame = new Uint8Array(4 + messageData.length);
        
        // Write length prefix (little-endian uint32)
        const view = new DataView(frame.buffer);
        view.setUint32(0, messageData.length, true);
        
        // Write message data
        frame.set(messageData, 4);
        
        ws.send(frame);
    }

    /**
     * Handle incoming WebSocket message
     * @param {ArrayBuffer|Blob} data - Raw WebSocket data
     */
    function handleMessage(data) {
        if (data instanceof ArrayBuffer) {
            processBinaryMessage(new Uint8Array(data));
        } else {
            // Blob handling for browser compatibility
            const reader = new FileReader();
            reader.onload = () => {
                processBinaryMessage(new Uint8Array(reader.result));
            };
            reader.readAsArrayBuffer(data);
        }
    }

    /**
     * Process binary message with length-prefixed framing
     * @param {Uint8Array} data - Raw binary data
     */
    function processBinaryMessage(data) {
        let offset = 0;

        while (offset < data.length) {
            if (pendingBuffer === null) {
                // Start of new message
                if (offset + 4 > data.length) {
                    // Not enough data for length header
                    pendingBuffer = data.slice(offset);
                    pendingLength = -1;
                    break;
                }

                const length = new DataView(
                    data.buffer,
                    data.byteOffset + offset,
                    4
                ).getUint32(0, true);
                offset += 4;

                if (offset + length > data.length) {
                    // Not enough data for full message
                    pendingBuffer = data.slice(offset - 4);
                    pendingLength = length;
                    break;
                }

                const messageData = data.slice(offset, offset + length);
                offset += length;
                deliverMessage(messageData);
            } else {
                // Continuation of pending message
                if (pendingLength === -1) {
                    // Still need length header
                    const needed = 4 - pendingBuffer.length;
                    if (data.length - offset < needed) {
                        pendingBuffer = new Uint8Array([
                            ...pendingBuffer,
                            ...data.slice(offset)
                        ]);
                        break;
                    }

                    const tempBuffer = new Uint8Array(
                        pendingBuffer.length + needed
                    );
                    tempBuffer.set(pendingBuffer);
                    tempBuffer.set(
                        data.slice(offset, offset + needed),
                        pendingBuffer.length
                    );

                    pendingLength = new DataView(
                        tempBuffer.buffer
                    ).getUint32(0, true);
                    pendingBuffer = null;
                    offset += needed;
                } else {
                    // Have length, need message body
                    const needed = pendingLength - pendingBuffer.length;
                    if (data.length - offset < needed) {
                        pendingBuffer = new Uint8Array([
                            ...pendingBuffer,
                            ...data.slice(offset)
                        ]);
                        break;
                    }

                    const messageData = new Uint8Array(pendingLength);
                    messageData.set(pendingBuffer);
                    messageData.set(
                        data.slice(offset, offset + needed),
                        pendingBuffer.length
                    );

                    offset += needed;
                    pendingBuffer = null;
                    deliverMessage(messageData);
                }
            }
        }
    }

    /**
     * Deliver a complete message to Elm — raw passthrough.
     *
     * processBinaryMessage() has ALREADY stripped the length prefix (it
     * reassembles [len][msg] frames that may split across WS messages).
     * Elm must NOT strip it again: doing so misreads the capnp
     * segment-count word as a frame length and corrupts every inbound
     * message. Framing stays symmetric: applied once on send (JS),
     * stripped once on receive (JS).
     * @param {Uint8Array} messageData - Complete message data
     */
    function deliverMessage(messageData) {
        ports.wsReceive.send({
            data: Array.from(messageData)
        });
    }

    // Subscribe to outgoing ports
    if (ports.wsConnect) {
        ports.wsConnect.subscribe(connect);
    }
    if (ports.wsDisconnect) {
        ports.wsDisconnect.subscribe(disconnect);
    }
    if (ports.wsSend) {
        ports.wsSend.subscribe(send);
    }

    return {
        connect,
        disconnect,
        send,
        isConnected: () => ws && ws.readyState === WebSocket.OPEN
    };
}

/**
 * Initialize WebSocket ports for an Elm app
 * 
 * @param {Object} app - Elm app instance with ports
 * @param {Object} options - Configuration options
 * @returns {Object} WebSocket manager
 */
export function initWebSocketPorts(app, options = {}) {
    return createWebSocketManager(app.ports, options);
}

// Default export for CommonJS compatibility
export default {
    createWebSocketManager,
    initWebSocketPorts
};
