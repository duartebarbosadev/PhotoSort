# todo:

- Notify if there's changes to the folder (but do not notify if it was the user that deleted an image) and refresh if there is and keep similar position in view
- Scrolling up with the option to skip single clusters will not go to the last item of a group but instead go to the first item and is wrong

Local similarity save model, work without internet 


stop logging this kinds of stuff 
2025-12-24 12:05:25 - DEBUG    - [ui.main_window] - [main_window.py:3328] - eventFilter: key=16777249, modifiers=['Ctrl'], search_has_focus=False
2025-12-24 12:05:25 - DEBUG    - [ui.main_window] - [main_window.py:3417] - Key with other modifiers detected (passing to default handler): 16777249, modifiers: KeyboardModifier.ControlModifier

Whenever we search in clusters and delete the search the clusters are all minimized.

Add more to the analyze best image, make it do faster stuff before like analyze like sharpness detection etc


whenever a folder is selected, make on the preview image display like a grid of all images in that folder

Todo - draw your own like workflow for organizing folders like
new folder per date, then subfolder is per face then etc
but user can draw their own workflow and save it as a template for future use
or the inverse like just per face then per date

create a tutorial that shows shortcuts like mark for deletion etc
add on readme my workflow of just scrolling and pressing D and shift D

Overlap one image on top of other with a bar in the middle and let user change bar position to look at differences between the two images, like a before and after of edits or something like that


* **Enhanced Search Capabilities**:
  * Search by EXIF metadata (camera model, settings, date ranges)
* **Advanced AI Object/Scene Detections & Grouping**:
  * **Car Model Recognition**: Identify and allow grouping by specific car models in photos.
  * **Face Recognition/Clustering**: Detect faces and group photos by the people present.

Separate --clear-cache to clear models cache commands 


On the 1. Organize, add an option to hide non image files

Add intellisearch with clip or something like dog photos 

On the 1. Organize, if the user drags a photo or multiple like dsadsa.jpg but there's a same name file like dsadsa.xmp ask the user if they want to move the xmp file as well, and if they want that question to be remembered for any kind of future scenario

Change organize to be last option (since a lot of images will be deleted with the culling and pick best process)



Now I want you to create a new step, after 1. and before the cull process
Where its to delete easy images
Images that are really blurry, images that are like black or just white (for instance images that the user took with the lens cover on), images 
This needs to be a page where the user can mark for deletion images that are 100% sent to deletion
The UI can be just like the culling but for single images.
When there's duplicate images that are equal like 99.999% or trully equal it should also display there for the user to delete at least one of them
It should suggest the user which one to remove and which one to keep and this would be based on who has more exif data and size (the one without or less exif data and smaller would be suggested to be deleted) (Deleted as in marked to delete just like the cull option)
When the user leaves the mode of deleting easy images and goes to the cull process, the images that were marked for deletion in the easy delete step should not be taken into consideration in the pick best process.
