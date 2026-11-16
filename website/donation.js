const donationAddress = document.querySelector('#donation-address');
const copyAddressButton = document.querySelector('.copy-address');
const copyAddressStatus = document.querySelector('.copy-address-status');

copyAddressButton.addEventListener('click', async () => {
  copyAddressButton.disabled = true;

  try {
    await navigator.clipboard.writeText(donationAddress.textContent.trim());
    copyAddressStatus.textContent = 'Address copied.';
  } catch {
    copyAddressStatus.textContent = 'Couldn’t copy. Select the address and copy it manually.';
  } finally {
    copyAddressButton.disabled = false;
  }
});

copyAddressButton.hidden = false;
