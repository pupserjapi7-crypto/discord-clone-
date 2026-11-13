import { expect, test } from '@playwright/test';

test('register, login, guild, channel, message, second client, reconnect', async ({
  browser,
  request,
}) => {
  const suffix = `${Date.now().toString(36)}`;
  const handleA = `smokea${suffix}`;
  const handleB = `smokeb${suffix}`;
  const password = 'smoke-password-123';
  const message = `hallo ${suffix}`;
  const reply = `antwort ${suffix}`;

  // Fixture data through the API: two fresh accounts, one guild with an
  // invite, so the test works on an empty production database too.
  const register = async (handle: string) => {
    const res = await request.post('/api/v1/auth/register', {
      data: { handle, password },
    });
    expect(res.ok(), await res.text()).toBeTruthy();
    return (await res.json()).access_token as string;
  };
  const tokenA = await register(handleA);
  const tokenB = await register(handleB);
  const authA = { authorization: `Bearer ${tokenA}` };
  const authB = { authorization: `Bearer ${tokenB}` };

  const guildRes = await request.post('/api/v1/guilds', {
    headers: authA,
    data: { name: `Smoke ${suffix}` },
  });
  expect(guildRes.ok(), await guildRes.text()).toBeTruthy();
  const guild = await guildRes.json();

  const inviteRes = await request.post(`/api/v1/guilds/${guild.id}/invites`, {
    headers: authA,
  });
  expect(inviteRes.ok(), await inviteRes.text()).toBeTruthy();
  const { code } = await inviteRes.json();

  const joinRes = await request.post(`/api/v1/invites/${code}`, {
    headers: authB,
  });
  expect(joinRes.ok(), await joinRes.text()).toBeTruthy();

  const ctxA = await browser.newContext();
  const pageA = await ctxA.newPage();
  await pageA.goto('/');

  // 1. The registered account can log in through the UI.
  await pageA.getByLabel('Handle').fill(handleA);
  await pageA.getByLabel('Passwort').fill(password);
  await pageA.getByRole('button', { name: 'Anmelden' }).click();
  await expect(pageA.getByText('verbunden')).toBeVisible();
  await expect(pageA.getByRole('log')).toBeVisible();

  // 2. Send a message in the auto-selected channel.
  await pageA.getByLabel('Nachricht').fill(message);
  await pageA.getByLabel('Nachricht').press('Enter');
  await expect(pageA.getByRole('log').getByText(message)).toBeVisible();

  // 3. A second independent client sees the message after login.
  const ctxB = await browser.newContext();
  const pageB = await ctxB.newPage();
  await pageB.goto('/');
  await pageB.getByLabel('Handle').fill(handleB);
  await pageB.getByLabel('Passwort').fill(password);
  await pageB.getByRole('button', { name: 'Anmelden' }).click();
  await expect(pageB.getByRole('log').getByText(message)).toBeVisible();

  // 4. Drop the network on client A; the gateway must resume the session and
  //    traffic from B must arrive afterwards.
  await ctxA.setOffline(true);
  await pageA.waitForTimeout(1_000);
  await ctxA.setOffline(false);
  await expect(pageA.getByText('verbunden')).toBeVisible({ timeout: 20_000 });

  await pageB.getByLabel('Nachricht').fill(reply);
  await pageB.getByLabel('Nachricht').press('Enter');
  await expect(pageA.getByRole('log').getByText(reply)).toBeVisible({ timeout: 20_000 });

  await ctxA.close();
  await ctxB.close();
});
