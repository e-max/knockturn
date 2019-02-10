# KnockturnAllee Payment flow

Current version supports customer-initiated payments, invoice support will be added in the next revision.

## Create order
Merchant's backend sends a request to KA to register an order, uisng API token
```
POST /payments/{merchantId}

{
	orderId: "xyz",
	amount : 1000,
	currency: "USD",
	confirmations: 10,
	callbackUrl: "https://xyz.com/22"
	email: "user@domain.com"
	...
}
```

In case of success KA returns 201. KA calculates amount in grins and expiration time, set status `UNPAID` and saves order in DB. KA send emails to a customer if an address is provided. 

## Get order status
After that a customer is redirected to an order status page. This request may be made multiple times to get the current state of the order.
```
GET /payments/{merchantId}/{orderId}
```

KA returns a human-readable HTML page with a payment URL `/payments/{merchantId}/{orderId}`
, amount in grins status of the order (initially UNPAID) and additional information. e.g. expiration time for this invoice.

## Payment
Customers initiates a payment using her grin wallet to URL from the order status page
```
POST  /payments/{merchantId}/{orderId}
{Grin Tx slate}
```

KA acts as a proxy for Grin wallet. Before forwarding a request it checks that order exists, otherwise returns 404.
It checks that amount equals to the order amount and status is UNPAID or REJECTED.  After receiving a response from the wallet KA updates status of the payment (RECEIVED or REJECTED).

## Confirmation
KA polls the wallet to get updates reg on-chain status of transaction, this information is available via `Get order status` request. When the tx gets required number of confirmations KA sent a request to `callbackUrl` and sends an emails to the customer.
