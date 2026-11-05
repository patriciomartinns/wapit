const send = (code, message, details) => ({
  caption: 'POST /v23.0/100000000000002/messages',
  status: '400',
  failed: true,
  body: {
    error: {
      message,
      type: 'OAuthException',
      code,
      error_data: { messaging_product: 'whatsapp', details },
      fbtrace_id: 'A3kQ8x…',
    },
  },
});

export const errors = {
  132000: send(
    132000,
    '(#132000) Number of parameters does not match the expected number of params',
    'body: number of localizable_params (2) does not match the expected number of params (3)',
  ),
  132001: send(
    132001,
    '(#132001) Template name does not exist in the translation',
    'template name (order_ready) does not exist in pt_BR',
  ),
  132005: send(
    132005,
    '(#132005) Translated text too long',
    'body: text after variable substitution exceeds 1024 characters',
  ),
  132012: send(
    132012,
    '(#132012) Parameter format does not match format in the created template',
    'header: Format mismatch, expected IMAGE, received TEXT',
  ),
  132015: send(132015, '(#132015) Template is Paused', 'template order_ready is paused'),
  132018: send(
    132018,
    '(#132018) There was an issue with the parameters in your template.',
    'body: param text cannot have new-line/tab characters or more than 4 consecutive spaces',
  ),
  131009: send(
    131009,
    '(#131009) Parameter value is not valid',
    'buttons: button index 2 is out of range (template has 2 buttons)',
  ),
  131047: {
    caption: 'POST localhost:8000/webhook',
    status: 'failed',
    failed: true,
    body: {
      object: 'whatsapp_business_account',
      entry: [
        {
          id: '200000000000002',
          changes: [
            {
              field: 'messages',
              value: {
                messaging_product: 'whatsapp',
                statuses: [
                  {
                    id: 'wamid.HBgM…',
                    status: 'failed',
                    recipient_id: '15550000000',
                    errors: [
                      {
                        code: 131047,
                        title: 'Re-engagement message',
                        error_data: {
                          details: 'More than 24 hours have passed since the recipient last replied to the sender number.',
                        },
                      },
                    ],
                  },
                ],
              },
            },
          ],
        },
      ],
    },
  },
};
