import { expect, test } from '@playwright/test';

test('register, login, guild, channel, message, second client, reconnect', async ({ browser }) => {
  const suffix = `${Date.now().toString(36)}`;
  const handle = `smoke${suffix}`;
  const message = `hallo ${suffix}`;
  const reply = `antwort ${suffix}`;

  const ctxA = await browser.newContext();
  const pageA = await ctxA.newPage();
  await pageA.goto('/');

  // 1. Register a brand new account through the UI.
  await pageA.getByRole('button', { name: 'Neues Konto erstellen' }).click();
  await pageA.getByLabel('Handle').fill(handle);
  await pageA.getByLabel('Passwort').fill('smoke-password-123');
  await pageA.getByRole('button', { name: 'Registrieren' }).click();
  await expect(pageA.getByRole('button', { name: 'Logout' })).toBeVisible();
  await pageA.getByRole('button', { name: 'Logout' }).click();

  // 2. Log in as a seeded member that already belongs to a guild.
  await pageA.getByLabel('Handle').fill('load0000');
  await pageA.getByLabel('Passwort').fill('loadtest-password');
  await pageA.getByRole('button', { name: 'Anmelden' }).click();
  await expect(pageA.getByText('verbunden')).toBeVisible();
  await expect(pageA.getByRole('log')).toBeVisible();

  // 3. Send a message in the auto-selected channel.
  await pageA.getByLabel('Nachricht').fill(message);
  await pageA.getByLabel('Nachricht').press('Enter');
  await expect(pageA.getByRole('log').getByText(message)).toBeVisible();

  // 4. A second independent client sees the message after login.
  const ctxB = await browser.newContext();
  const pageB = await ctxB.newPage();
  await pageB.goto('/');
  await pageB.getByLabel('Handle').fill('load0001');
  await pageB.getByLabel('Passwort').fill('loadtest-password');
  await pageB.getByRole('button', { name: 'Anmelden' }).click();
  await expect(pageB.getByRole('log').getByText(message)).toBeVisible();

  // 5. Drop the network on client A; the gateway must resume the session and
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
