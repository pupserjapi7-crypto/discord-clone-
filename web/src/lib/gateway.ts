import { decode, encode } from '@msgpack/msgpack';
import type { Message } from './types';

function normalizeMessage(raw: Message): Message {
  return {
    ...raw,
    id: String(raw.id),
    channel_id: String(raw.channel_id),
    reply_to: raw.reply_to == null ? null : String(raw.reply_to),
    author: { ...raw.author, id: String(raw.author.id) }
  };
}

const OP_HELLO = 1;
const OP_HELLO_OK = 2;
const OP_RESUMED = 4;
const OP_INVALID_SESSION = 5;
const OP_DISPATCH = 8;
const OP_ACK = 12;
const OP_ERROR = 13;
const OP_REQUEST = 9;
const OP_RESYNC = 15;

type Handlers = {
  onMessage: (message: Message) => void;
  onAck: (nonce: number, id: number) => void;
  onStatus: (status: 'connecting' | 'online' | 'offline') => void;
  onError: (message: string) => void;
  onResync?: (oldestSeq: number) => void;
};

export class Gateway {
  private ws: WebSocket | null = null;
  private token = '';
  private channelId: string | null = null;
  private sessionId: string | null = null;
  private lastSeq = 0;
  private retries = 0;
  private heartbeat: number | undefined;
  private closed = false;

  constructor(private handlers: Handlers) {}

  connect(token: string, channelId: string) {
    this.token = token;
    this.channelId = channelId;
    this.closed = false;
    this.open();
  }

  setChannel(channelId: string) {
    this.channelId = channelId;
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.send(OP_REQUEST, { t: 'subscribe', d: { channel_id: channelId } });
    }
  }

  sendMessage(nonce: number, channelId: string, content: string) {
    this.send(OP_REQUEST, {
      t: 'message_create',
      d: {
        nonce,
        channel_id: channelId,
        content,
        reply_to: null,
        attachment_ids: []
      }
    });
  }

  close() {
    this.closed = true;
    window.clearInterval(this.heartbeat);
    this.ws?.close();
    this.ws = null;
  }

  private open() {
    this.handlers.onStatus('connecting');
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${scheme}://${location.host}/gateway`);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;

    ws.onopen = () => {
      const resume = this.sessionId
        ? { session_id: this.sessionId, last_seq: this.lastSeq }
        : null;
      this.send(OP_HELLO, { t: 'hello', d: { token: this.token, resume } });
    };

    ws.onmessage = (event) => {
      if (typeof event.data === 'string') return;
      this.handle(new Uint8Array(event.data as ArrayBuffer));
    };

    ws.onclose = () => {
      this.handlers.onStatus('offline');
      window.clearInterval(this.heartbeat);
      if (!this.closed) {
        const delay = Math.min(5000, 300 * 2 ** this.retries++);
        window.setTimeout(() => this.open(), delay);
      }
    };
  }

  private send(op: number, payload: unknown) {
    if (this.ws?.readyState !== WebSocket.OPEN) return;
    const body = encode(payload);
    const frame = new Uint8Array(10 + body.length);
    frame[0] = op;
    frame[1] = 0;
    const view = new DataView(frame.buffer);
    view.setUint32(2, 0, false);
    view.setUint32(6, body.length, false);
    frame.set(body, 10);
    this.ws.send(frame);
  }

  private handle(frame: Uint8Array) {
    const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
    const op = frame[0];
    const seq = view.getUint32(2, false);
    const length = view.getUint32(6, false);
    const body = frame.subarray(10, 10 + length);

    if (op === OP_RESYNC) {
      const resync = decode(body) as { d?: { oldest_seq?: number } };
      const oldestSeq = resync.d?.oldest_seq ?? 0;
      this.lastSeq = Math.max(0, oldestSeq - 1);
      this.handlers.onResync?.(oldestSeq);
      return;
    }

    if (op === OP_DISPATCH) {
      this.lastSeq = Math.max(this.lastSeq, seq);
      const event = decode(body) as { t: string; d?: { message?: Message } };
      if ((event.t === 'message_create' || event.t === 'message_update') && event.d?.message) {
        this.handlers.onMessage(normalizeMessage(event.d.message));
      }
      return;
    }

    const payload = decode(body) as {
      t: string;
      d?: {
        session_id?: string;
        nonce?: number;
        id?: number;
        message?: string;
      };
    };

    if (op === OP_HELLO_OK) {
      this.sessionId = payload.d?.session_id ?? null;
      this.retries = 0;
      this.handlers.onStatus('online');
      this.startHeartbeat();
      if (this.channelId !== null) {
        this.send(OP_REQUEST, { t: 'subscribe', d: { channel_id: this.channelId } });
      }
    } else if (op === OP_RESUMED) {
      this.retries = 0;
      this.handlers.onStatus('online');
      this.startHeartbeat();
      if (this.channelId !== null) {
        this.send(OP_REQUEST, { t: 'subscribe', d: { channel_id: this.channelId } });
      }
    } else if (op === OP_INVALID_SESSION) {
      this.sessionId = null;
      this.lastSeq = 0;
    } else if (op === OP_ACK) {
      if (payload.d?.nonce !== undefined && payload.d.id !== undefined) {
        this.handlers.onAck(payload.d.nonce, payload.d.id);
      }
    } else if (op === OP_ERROR) {
      this.handlers.onError(payload.d?.message ?? 'gateway error');
    }
  }

  private startHeartbeat() {
    window.clearInterval(this.heartbeat);
    this.heartbeat = window.setInterval(() => {
      this.send(OP_REQUEST, { t: 'heartbeat', d: { seq: this.lastSeq } });
    }, 30000);
  }
}
