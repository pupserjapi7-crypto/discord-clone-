import { expect, test } from '@playwright/test';

test('register, create guild, invite, chat, second client, reconnect', async ({
  browser,
  request,
}) => {
  const suffix = `${Date.now().toString(36)}`;
  const handleA = `smokea${suffix}`;
  const handleB = `smokeb${suffix}`;
  const password = 'smoke-password-123';
  const message = `hallo ${suffix}`;
  const reply = `antwort ${suffix}`;

  const ctxA = await browser.newContext();
  const pageA = await ctxA.newPage();
  await pageA.goto('/');

  // 1. Register a brand new account and create the first server through the UI.
  await pageA.getByRole('button', { name: 'Neues Konto erstellen' }).click();
  await pageA.getByLabel('Handle').fill(handleA);
  await pageA.getByLabel('Passwort').fill(password);
  await pageA.getByRole('button', { name: 'Registrieren' }).click();
  await expect(pageA.getByRole('button', { name: 'Logout' })).toBeVisible();

  await pageA.getByLabel('Servername').fill(`Smoke ${suffix}`);
  await pageA.getByRole('button', { name: 'Erstellen', exact: true }).click();
  await expect(pageA.getByText('verbunden')).toBeVisible();
  await expect(pageA.getByRole('log')).toBeVisible();

  // 2. Send a message in the auto-selected channel.
  await pageA.getByLabel('Nachricht').fill(message);
  await pageA.getByLabel('Nachricht').press('Enter');
  await expect(pageA.getByRole('log').getByText(message)).toBeVisible();

  // 3. Create an invite in the UI and let a second fresh account join it.
  await pageA.getByRole('button', { name: 'Einladen' }).click();
  const codeEl = pageA.getByTestId('invite-code');
  await expect(codeEl).toBeVisible();
  const code = (await codeEl.textContent())!.trim();
  expect(code.length).toBeGreaterThan(4);

  const register = await request.post('/api/v1/auth/register', {
    data: { handle: handleB, password },
  });
  expect(register.ok(), await register.text()).toBeTruthy();
  const tokenB = (await register.json()).access_token as string;
  const joined = await request.post(`/api/v1/invites/${code}`, {
    headers: { authorization: `Bearer ${tokenB}` },
  });
  expect(joined.ok(), await joined.text()).toBeTruthy();

  // 4. The second client logs in through the UI and sees the message.
  const ctxB = await browser.newContext();
  const pageB = await ctxB.newPage();
  await pageB.goto('/');
  await pageB.getByLabel('Handle').fill(handleB);
  await pageB.getByLabel('Passwort').fill(password);
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
