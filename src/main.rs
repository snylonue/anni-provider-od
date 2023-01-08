use std::num::NonZeroU8;

use anni_provider_od::OneDriveProvider;
use onedrive_api::{DriveId, DriveLocation, OneDrive};

#[tokio::main]
async fn main() {
    let drive = OneDrive::new("eyJ0eXAiOiJKV1QiLCJub25jZSI6IlhlczZla3lMbFNtdklFNkZKcnl3c3pVbUk1NlktbmlfRGN0ajNYeVVDUFEiLCJhbGciOiJSUzI1NiIsIng1dCI6Ii1LSTNROW5OUjdiUm9meG1lWm9YcWJIWkdldyIsImtpZCI6Ii1LSTNROW5OUjdiUm9meG1lWm9YcWJIWkdldyJ9.eyJhdWQiOiIwMDAwMDAwMy0wMDAwLTAwMDAtYzAwMC0wMDAwMDAwMDAwMDAiLCJpc3MiOiJodHRwczovL3N0cy53aW5kb3dzLm5ldC9hMDg4NzNmNS1lN2IyLTRhNDgtYWEwYS1kNDgyYjcwNjc3ZDQvIiwiaWF0IjoxNjczMTY1MTU2LCJuYmYiOjE2NzMxNjUxNTYsImV4cCI6MTY3MzE3MDM0MiwiYWNjdCI6MCwiYWNyIjoiMSIsImFpbyI6IkFUUUF5LzhUQUFBQUcrQ2M0ZmloMUJVaytBdE8wT2srdE13OTJmYjh6TzFzQlJGa1UrSkprNHpvNytiVzlsbmpxZklzcS9wdHJ3djEiLCJhbXIiOlsicHdkIl0sImFwcF9kaXNwbGF5bmFtZSI6IkFubmlsU2VydmVybGVzcyIsImFwcGlkIjoiY2RiODM4NzAtNmUwNC00YmVkLWI3NDQtMTcwNDYwNDVlYzdkIiwiYXBwaWRhY3IiOiIxIiwiZmFtaWx5X25hbWUiOiJzbnlsb251ZSIsImdpdmVuX25hbWUiOiJtYXNoaXJvIiwiaWR0eXAiOiJ1c2VyIiwiaXBhZGRyIjoiMTE0LjIyMC4yOC45NCIsIm5hbWUiOiJzbnlsb251ZSBtYXNoaXJvIiwib2lkIjoiOTZmYjEzNGMtZmEzMC00ZThhLTk3NDUtYzAwY2UwNWJhYzIyIiwicGxhdGYiOiIzIiwicHVpZCI6IjEwMDMyMDAyNjNDMkJBRTciLCJyaCI6IjAuQVVvQTlYT0lvTExuU0VxcUN0U0N0d1ozMUFNQUFBQUFBQUFBd0FBQUFBQUFBQUNKQUIwLiIsInNjcCI6IkZpbGVzLlJlYWQgRmlsZXMuUmVhZC5BbGwgRmlsZXMuUmVhZFdyaXRlIEZpbGVzLlJlYWRXcml0ZS5BbGwgU2l0ZXMuUmVhZC5BbGwgcHJvZmlsZSBvcGVuaWQgZW1haWwiLCJzdWIiOiJyT1VzZG1ETWJ4U29XRlZCYWt0THdJRzF3UmR4TzNKOHdlSXJyanFmanVnIiwidGVuYW50X3JlZ2lvbl9zY29wZSI6IkFTIiwidGlkIjoiYTA4ODczZjUtZTdiMi00YTQ4LWFhMGEtZDQ4MmI3MDY3N2Q0IiwidW5pcXVlX25hbWUiOiJzbnlsb251ZUBzbnlsb251ZS5vbm1pY3Jvc29mdC5jb20iLCJ1cG4iOiJzbnlsb251ZUBzbnlsb251ZS5vbm1pY3Jvc29mdC5jb20iLCJ1dGkiOiJEUFlhSjhyd1cwcUJ0cGRoRkQwbUFBIiwidmVyIjoiMS4wIiwid2lkcyI6WyI2MmU5MDM5NC02OWY1LTQyMzctOTE5MC0wMTIxNzcxNDVlMTAiLCJiNzlmYmY0ZC0zZWY5LTQ2ODktODE0My03NmIxOTRlODU1MDkiXSwieG1zX3N0Ijp7InN1YiI6IlJRZXJLelNhRk9YNHVBVDRueFZSd1hVQ3V2YVBYdllFbld4MThWaGd1TTQifSwieG1zX3RjZHQiOjE2NzMxNTAyNjZ9.Sx9fpE6ebIFid5KanITS-eY9_ROs8MOOl4VGmCRns1u1tN0LppJXhQHwpLlmKHCzKGrz4Kun1vcbk9-iWEpuwjkfhtewpMPnlmBd0YVR0W1MbZq2VgWYJKUlHUUrFoV13_YrAlpKOfNJfZWtU59ZoCW8BFNCUlNrV_u0dL0KDFhCFHN2BHOoifeC66lyPndn_PfiQ9n49kCG9EdOqvlMAOwz-C4nlA8frYL6KzPbchuwHw_h9LljDihj7iuvl1FksPxzIpl3YvqPlCqYpeY4PblcMB6lWel0kta5CN3ljlMZbgFVHB2mPrmOwYuMFdroQmD3jyed1XE4co_jR9x5kA", DriveLocation::from_id(DriveId(String::from("b!uyGkzZXn6UeUrlI00cEEwB0U-PTBJVNIkX2vruaA2Wsnkoejm3etQpoha4pffHk9"))));
    let mut provider = OneDriveProvider::new(drive);
    provider.reload_albums().await.unwrap();
    let url = unsafe {
        provider
            .audio_url(
                "1178e13b-f661-49db-98db-28a04c5583b7",
                NonZeroU8::new_unchecked(1),
                NonZeroU8::new_unchecked(1),
            )
            .await
            .unwrap()
            .0
    };
    println!("{url}");
}
