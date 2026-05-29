const { expect, test } = require('@playwright/test');

function uniqueLandlord() {
  const stamp = Date.now();
  return {
    email: `playwright-landlord-${stamp}@example.com`,
    fullName: `Playwright Landlord ${stamp}`
  };
}

async function clickLatestButton(page, name) {
  const buttons = page.getByRole('button', { name });
  await expect(buttons.last()).toBeVisible();
  await buttons.last().click();
}

test('adding a landlord persists and reloads the latest landlord list', async ({ page, baseURL }) => {
  const landlord = uniqueLandlord();

  await page.goto(`${baseURL}?pw=${Date.now()}`);
  await clickLatestButton(page, /Open as Admin/i);
  await clickLatestButton(page, /^Landlord$/i);
  await clickLatestButton(page, /Add Landlord/i);

  await page.getByLabel('Email').last().fill(landlord.email);
  await page.getByLabel('Full Name').last().fill(landlord.fullName);
  await clickLatestButton(page, /^Submit$/i);

  await expect(page.getByText(landlord.email)).toBeVisible();
  await expect(page.getByText(landlord.fullName)).toBeVisible();

  const transcriptText = await page.locator('body').innerText();
  const listPosition = transcriptText.lastIndexOf('Landlords');
  const landlordPosition = transcriptText.lastIndexOf(landlord.email);
  expect(landlordPosition).toBeGreaterThan(listPosition);

  await page.getByText(landlord.email).last().click();
  await expect(page.getByLabel('Email').last()).toHaveValue(landlord.email);
});
