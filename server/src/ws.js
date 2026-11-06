// ws.js — WebSocket（规格 §4）。路径 /ws?token=
'use strict';

const { WebSocketServer } = require('ws');
const { moderate } = require('./moderation');

const HEARTBEAT_MISS_MS = 60_000; // 60s 未收到任何帧 → 判离线
const HEARTBEAT_SCAN_MS = 15_000;

function log(...args) {
  console.log(`[WS ${new Date().toISOString()}]`, ...args);
}

class WSHub {
  /**
   * @param {import('./store').Store} store
   */
  constructor(store) {
    this.store = store;
    /** @type {Map<string, Set<import('ws').WebSocket>>} anon_id -> connections */
    this.conns = new Map();
    this.wss = new WebSocketServer({ noServer: true });
    this.wss.on('connection', (ws, req) => this._onConnection(ws, req));
    this._scanTimer = setInterval(() => this._scan(), HEARTBEAT_SCAN_MS);
    this._scanTimer.unref?.();
  }

  handleUpgrade(req, socket, head) {
    // 规格 §4.1：连接即鉴权——先完成握手，连接后由 _onConnection 校验 token；
    // 无效 token → 发送 error 帧并以 close code 4001 关闭。
    this.wss.handleUpgrade(req, socket, head, (ws) => {
      this.wss.emit('connection', ws, req);
    });
  }

  isOnline(anonId) {
    const set = this.conns.get(anonId);
    return !!set && set.size > 0;
  }

  sendTo(anonId, obj) {
    const set = this.conns.get(anonId);
    if (!set) return;
    const data = JSON.stringify(obj);
    for (const ws of set) {
      if (ws.readyState === ws.OPEN) ws.send(data);
    }
  }

  _addConn(anonId, ws) {
    let set = this.conns.get(anonId);
    if (!set) { set = new Set(); this.conns.set(anonId, set); }
    set.add(ws);
  }

  _removeConn(anonId, ws) {
    const set = this.conns.get(anonId);
    if (!set) return;
    set.delete(ws);
    if (set.size === 0) this.conns.delete(anonId);
  }

  _broadcastPresence(anonId, online) {
    const obj = { type: 'presence', who: anonId, online, ts: Math.floor(Date.now() / 1000) };
    const data = JSON.stringify(obj);
    for (const [id, set] of this.conns) {
      if (id === anonId) continue;
      for (const ws of set) if (ws.readyState === ws.OPEN) ws.send(data);
    }
  }

  _onConnection(ws, req) {
    // 升级时已验过 token；这里再取一次用户（升级回调内 req.url 仍可用）
    const url = new URL(req.url, 'http://localhost');
    const token = url.searchParams.get('token') || '';
    const user = this.store.tokenToUser(token);
    if (!user) {
      ws.send(JSON.stringify({ type: 'error', code: 'invalid_token', message: '无效 token' }));
      ws.close(4001);
      return;
    }
    const anonId = user.anon_id;
    ws._anonId = anonId;
    ws._lastActivity = Date.now();

    // 上线状态
    this.store.setStatus(anonId, 'online');
    this._addConn(anonId, ws);
    log(`open anon_id=${anonId} ip=${req.socket.remoteAddress}`);

    // hello
    const profile = this.store.rowToProfile(user, anonId);
    ws.send(JSON.stringify({ type: 'hello', anon_id: anonId, profile }));

    // inbox 离线补投
    const inbox = this.store.popInbox(anonId);
    if (inbox.length) {
      ws.send(JSON.stringify({ type: 'inbox', messages: inbox }));
      log(`inbox delivered=${anonId} count=${inbox.length}`);
    }

    // 广播 presence
    this._broadcastPresence(anonId, true);

    ws.on('message', (raw) => {
      ws._lastActivity = Date.now();
      let msg;
      try { msg = JSON.parse(String(raw)); } catch { return; }
      this._onMessage(ws, msg);
    });

    ws.on('close', () => this._removeAndMaybeOffline(ws));
    ws.on('error', () => this._removeAndMaybeOffline(ws));
  }

  _onMessage(ws, msg) {
    const anonId = ws._anonId;
    if (!msg || typeof msg !== 'object') return;

    if (msg.type === 'ping') {
      ws.send(JSON.stringify({ type: 'pong', ts: Math.floor(Date.now() / 1000) }));
      return;
    }

    if (msg.type === 'msg.send') {
      const to = String(msg.to || '');
      const body = String(msg.body ?? '').trim();
      log(`msg.send from=${anonId} to=${to} bytes=${body.length}`);

      if (!body) {
        ws.send(JSON.stringify({ type: 'error', code: 'invalid_body', message: '消息内容为空' }));
        return;
      }
      const target = this.store.getById(to);
      if (!target) {
        ws.send(JSON.stringify({ type: 'error', code: 'invalid_target', message: '目标用户不存在' }));
        return;
      }
      if (this.store.blockedEither(anonId, to)) {
        ws.send(JSON.stringify({ type: 'error', code: 'blocked', message: '对方已将你拉黑，消息未送达' }));
        log(`msg.send rejected blocked from=${anonId} to=${to}`);
        return;
      }
      const check = moderate(body);
      if (check.flagged) {
        ws.send(JSON.stringify({ type: 'error', code: 'censored', message: `消息被审核拦截（${check.reason}）` }));
        log(`msg.send censored from=${anonId} reason=${check.reason}`);
        return;
      }

      const delivered = this.isOnline(to) ? 1 : 0;
      const { mid, ts } = this.store.insertMessage(anonId, to, body, delivered);

      // 接收方在线 → 实时推送
      if (delivered) {
        this.sendTo(to, { type: 'msg.deliver', mid, from: anonId, body, ts });
      }
      // 发送方回执
      ws.send(JSON.stringify({ type: 'msg.deliver', mid, from: anonId, body, ts }));
      return;
    }
  }

  _removeAndMaybeOffline(ws) {
    const anonId = ws._anonId;
    if (!anonId) return;
    this._removeConn(anonId, ws);
    if (!this.conns.has(anonId)) {
      this.store.setStatus(anonId, 'offline');
      this._broadcastPresence(anonId, false);
      log(`close anon_id=${anonId} offline`);
    }
  }

  _scan() {
    const now = Date.now();
    for (const [, set] of this.conns) {
      for (const ws of set) {
        if (now - (ws._lastActivity || 0) > HEARTBEAT_MISS_MS) {
          try { ws.terminate(); } catch { /* 占位 */ }
          this._removeAndMaybeOffline(ws);
          log(`heartbeat timeout -> offline`);
        }
      }
    }
  }

  closeAll() {
    clearInterval(this._scanTimer);
    for (const [, set] of this.conns) {
      for (const ws of set) {
        try { ws.close(1001); } catch { /* 占位 */ }
      }
    }
    this.conns.clear();
  }
}

module.exports = { WSHub };
