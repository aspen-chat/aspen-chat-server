# The Privacy Policy and Terms of Use

A deployment's Privacy Policy and Terms of Use, each its administrators' Markdown, are pages anyone
may open, signed in or not. How the server keeps them:
`docs/architecture/administration/deployment-documents.md` at the repository's root.

## Where it lives

| Part                      | Where                                                                                                |
| ------------------------- | ---------------------------------------------------------------------------------------------------- |
| The pages                 | `DocumentScreen` (`src/features/documents/DocumentScreen.tsx`)                                       |
| Reading one, and its name | `useDeploymentDocument`, `documentNames` (`src/features/documents/documents.ts`)                     |
| Rendering the Markdown    | `DocumentText` (`src/features/documents/DocumentText.tsx`)                                           |
| The links                 | `DocumentLinks`, in `SignedOut` (`RootLayout.tsx`) and `AboutDialog`                                 |
| The dashboard's editor    | `DeploymentDocumentsSection` (`src/features/admin/DeploymentDocuments.tsx`)                          |
| Reading and writing       | `AspenClient.deploymentDocument`, `AdminApi.publishDeploymentDocument`, `withdrawDeploymentDocument` |

## The pages

- `/privacy` and `/terms`, routes of their own signed in, and screens `SignedOut` shows in place of
  the sign-in form signed out, with a way back to it.
- Signed in they show the home deployment's; signed out, the deployment the sign-in screen names.
- Each reads `GET /deployment/documents/{document}` when it opens, and shows when the document was
  last updated.
- A deployment without one shows `noPrivacyPolicy` or `noTermsOfUse` ("This deployment has not
  published a Privacy Policy."). There is no default text.

## The links

- The sign-in and create-account screens end with them, at the foot of the screen.
- About Aspen has them under the deployment's name, and closes Settings when one is followed.
- Both are offered whether or not the deployment has published them: the page says when it has not.

## Rendering

`DocumentText` renders GitHub-flavoured Markdown with `react-markdown`, apart from messages'
`Markdown`:

- A link may be named with words of its own (`[contact us](mailto:…)`), unlike in a message: the
  deployment's administrators wrote it for their own readers. Only web and mail addresses become
  links (`ExternalLink`, shared with messages).
- Pictures are not loaded, so reading the terms tells no other host the reader was there. The
  picture's description stands in its place.
- Raw HTML is ignored.
- Headings start one level below the page's title, so a document opening with `# Privacy Policy`
  makes no second top heading.
- Messages' nesting and table limits apply (`opensTooDeeply`, `remarkLimits`, `tableRow`), and a
  body that fails to render shows as its plain text.
- It uses the `message-body` styles, with `document-body` letting paragraphs flow as Markdown wraps
  them.

## The dashboard

Settings holds the editor, for holders of Manage deployment settings, below the profile:

- one field per document, starting from what is published, with Publish, Withdraw (when one is
  published), and a link to its page;
- the server refuses a blank body, and the field takes at most 65,536 characters, the server's
  limit;
- a withdrawn document's text stays in the field, so it can be published again.
