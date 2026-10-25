<?php
// ObserveCraft as a service provider of the local test IdP. SHA-256 signatures, like Entra ID's default
// (ObserveCraft rejects SHA-1 signed responses).
$metadata['http://localhost:5173/api/v1/auth/saml/default/metadata'] = array(
    'AssertionConsumerService' => 'http://localhost:5173/api/v1/auth/saml/default/acs',
    'signature.algorithm' => 'http://www.w3.org/2001/04/xmldsig-more#rsa-sha256',
);
