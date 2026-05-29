const { expect, test } = require('@playwright/test');

function uniqueSuffix() {
  return `${Date.now()}-${Math.floor(Math.random() * 100000)}`;
}

async function clickLatestButton(page, name) {
  const buttons = page.getByRole('button', { name });
  await expect(buttons.last()).toBeVisible();
  await buttons.last().click();
}

async function selectLatestChoice(page, label, preferredLabel) {
  const choice = page.getByLabel(label).last();
  await expect(choice).toBeVisible();
  try {
    await choice.selectOption({ label: preferredLabel });
  } catch (_) {
    await choice.selectOption({ index: 0 });
  }
}

test('complex SoRLa pack supports manager create, reload, detail, and child create flows', async ({ page, baseURL }) => {
  const suffix = uniqueSuffix();
  const landlord = {
    email: `complex-landlord-${suffix}@example.com`,
    fullName: `Complex Landlord ${suffix}`
  };
  const buildingAddress = `Complex House ${suffix}, 12 Example Street`;
  const unitNumber = `Unit ${suffix}`;
  const tenant = {
    email: `complex-tenant-${suffix}@example.com`,
    fullName: `Complex Tenant ${suffix}`,
    phone: `+447700${String(Math.floor(Math.random() * 1000000)).padStart(6, '0')}`
  };
  const tenantStartDate = '2026-06-01';
  const unitStartDate = '2026-07-01';

  await page.goto(`${baseURL}?pw=${suffix}`);
  await clickLatestButton(page, /Open as Admin/i);
  await clickLatestButton(page, /^Landlord$/i);
  await clickLatestButton(page, /Add Landlord/i);

  await page.getByLabel('Email').last().fill(landlord.email);
  await page.getByLabel('Full Name').last().fill(landlord.fullName);
  await clickLatestButton(page, /^Submit$/i);

  await expect(page.getByText(landlord.email)).toBeVisible();
  await expect(page.getByText(landlord.fullName)).toBeVisible();

  const listText = await page.locator('body').innerText();
  expect(listText.lastIndexOf(landlord.email)).toBeGreaterThan(listText.lastIndexOf('Landlords'));

  await page.getByText(landlord.email).last().click();
  await expect(page.getByLabel('Email').last()).toHaveValue(landlord.email);
  await expect(page.getByText('Buildings')).toBeVisible();

  await clickLatestButton(page, /Add Building/i);
  await expect(page.getByLabel(/Landlord Id/i)).toHaveCount(0);
  await page.getByLabel('Address').last().fill(buildingAddress);
  await clickLatestButton(page, /^Submit$/i);

  await expect(page.getByText(buildingAddress)).toBeVisible();
  await expect(page.getByText('Unable to submit this manager form')).toHaveCount(0);

  await page.getByText(buildingAddress).last().click();
  await expect(page.getByText('Units')).toBeVisible();
  await clickLatestButton(page, /Add Unit/i);
  await expect(page.getByLabel(/Building Id/i)).toHaveCount(0);
  await page.getByLabel('Unit Number').last().fill(unitNumber);
  await page.getByLabel('Rent Amount').last().fill('1250');
  await clickLatestButton(page, /^Submit$/i);

  await expect(page.getByText(unitNumber)).toBeVisible();
  await expect(page.getByText('Unable to submit this manager form')).toHaveCount(0);

  await page.goto(`${baseURL}?pw=${suffix}-tenant`);
  await clickLatestButton(page, /Open as Admin/i);
  await clickLatestButton(page, /^Tenant$/i);
  await clickLatestButton(page, /Add Tenant/i);
  await page.getByLabel('Email').last().fill(tenant.email);
  await page.getByLabel('Full Name').last().fill(tenant.fullName);
  await page.getByLabel('Phone').last().fill(tenant.phone);
  await clickLatestButton(page, /^Submit$/i);

  await expect(page.getByText(tenant.email)).toBeVisible();
  await expect(page.getByText(tenant.fullName)).toBeVisible();
  await page.getByText(tenant.email).last().click();
  await expect(page.getByText('Tenancies')).toBeVisible();
  await clickLatestButton(page, /Add Tenancy/i);
  await expect(page.getByLabel(/^Tenant Id/i)).toHaveCount(0);
  await selectLatestChoice(page, 'Unit Id', unitNumber);
  await page.getByLabel('Start Date').last().fill(tenantStartDate);
  await clickLatestButton(page, /^Submit$/i);

  await expect(page.getByText(tenantStartDate)).toBeVisible();
  await expect(page.getByText('Unable to submit this manager form')).toHaveCount(0);

  await page.goto(`${baseURL}?pw=${suffix}-unit`);
  await clickLatestButton(page, /Open as Admin/i);
  await clickLatestButton(page, /^Landlord$/i);
  await page.getByText(landlord.email).last().click();
  await page.getByText(buildingAddress).last().click();
  await page.getByText(unitNumber).last().click();
  await expect(page.getByText('Tenancies')).toBeVisible();
  await clickLatestButton(page, /Add Tenancy/i);
  await selectLatestChoice(page, 'Tenant Id', tenant.fullName);
  await page.getByLabel('Start Date').last().fill(unitStartDate);
  await clickLatestButton(page, /^Submit$/i);

  await expect(page.getByText(unitStartDate)).toBeVisible();
  await expect(page.getByText('Unable to submit this manager form')).toHaveCount(0);

  await page.goto(`${baseURL}?pw=${suffix}-metrics`);
  await clickLatestButton(page, /Open as Admin/i);
  await clickLatestButton(page, /^Metrics$/i);
  await expect(page.getByText('Metric').last()).toBeVisible();
  await expect(page.getByText('Value').last()).toBeVisible();
  await expect(page.getByText('Total Tenants').last()).toBeVisible();
  await expect(page.getByText('Active Tenancies').last()).toBeVisible();
  await clickLatestButton(page, /Total Tenants/i);
  await expect(page.getByText('Metric: total_tenants').last()).toBeVisible();
  await expect(page.getByText('Value').last()).toBeVisible();
});
