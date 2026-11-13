import type { UserPublic } from './types';

let token: string | null = null;
export const getToken = () => token;
export const setToken = (value: string | null) => {
  token = value;
};

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body) headers.set('content-type', 'application/json');
  if (token) headers.set('authorization', `Bearer ${token}`);

  const response = await fetch(path, { ...init, headers, credentials: 'include' });
  if (!response.ok) {
    let message = `HTTP ${response.status}`;
    try {
      const body = await response.json();
      message = body.message ?? message;
    } catch {
      /* keep status text */
    }
    throw new Error(message);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export const api = request;

export const auth = {
  async login(handle: string, password: string): Promise<UserPublic> {
    const result = await request<{ user: UserPublic; access_token: string }>(
      '/api/v1/auth/login',
      { method: 'POST', body: JSON.stringify({ handle, password }) }
    );
    token = result.access_token;
    return result.user;
  },

  async register(handle: string, password: string): Promise<UserPublic> {
    const result = await request<{ user: UserPublic; access_token: string }>(
      '/api/v1/auth/register',
      { method: 'POST', body: JSON.stringify({ handle, password }) }
    );
    token = result.access_token;
    return result.user;
  },

  async refresh(): Promise<UserPublic> {
    const result = await request<{ user: UserPublic; access_token: string }>(
      '/api/v1/auth/refresh',
      { method: 'POST' }
    );
    token = result.access_token;
    return result.user;
  },

  async logout(): Promise<void> {
    await request('/api/v1/auth/logout', { method: 'POST' });
    token = null;
  }
};
